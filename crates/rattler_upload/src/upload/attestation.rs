use miette::IntoDiagnostic;
use reqwest::header;
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use tempfile::NamedTempFile;
use tokio::process::Command as AsyncCommand;

use crate::upload::package::sha256_sum;

/// In-toto Statement structure for conda packages
#[derive(Debug, Serialize, Deserialize)]
pub struct Statement {
    #[serde(rename = "_type")]
    pub statement_type: String,
    pub subject: Vec<Subject>,
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    pub predicate: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Subject {
    pub name: String,
    pub digest: DigestSet,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DigestSet {
    pub sha256: String,
}

/// Response from GitHub attestation API
#[derive(Debug, Serialize, Deserialize)]
pub struct AttestationResponse {
    pub id: String,
}

/// Configuration for attestation creation
#[derive(Debug, Clone)]
pub struct AttestationConfig {
    pub repo_owner: String,
    pub repo_name: String,
    pub github_token: String,
    pub use_github_oidc: bool,
}

/// Create and store an attestation for a conda package using cosign
///
/// This function:
/// 1. Creates an in-toto statement for the package
/// 2. Uses cosign to sign the statement with GitHub OIDC or other identity
/// 3. Stores the signed attestation to GitHub's attestation API
pub async fn create_attestation_with_cosign(
    package_path: &Path,
    channel_url: &str,
    config: &AttestationConfig,
    client: &ClientWithMiddleware,
) -> miette::Result<String> {
    // Check if cosign is installed
    check_cosign_installed().await?;

    // Step 1: Create the in-toto statement
    let statement = create_intoto_statement(package_path, channel_url).await?;

    // Step 2: Sign with cosign
    let bundle_json = sign_with_cosign(&statement, package_path, config).await?;

    // Step 3: Store to GitHub
    let attestation_id = store_attestation_to_github(
        &bundle_json,
        &config.github_token,
        &config.repo_owner,
        &config.repo_name,
        client,
    )
    .await?;

    Ok(attestation_id)
}

/// Check if cosign is installed and available
async fn check_cosign_installed() -> miette::Result<()> {
    let output = AsyncCommand::new("cosign")
        .arg("version")
        .output()
        .await
        .into_diagnostic()
        .map_err(|_| {
            miette::miette!(
                "cosign is not installed or not found in PATH.\n\
             Install it with: pixi global install cosign"
            )
        })?;

    if !output.status.success() {
        return Err(miette::miette!(
            "cosign command failed. Please ensure cosign is properly installed.\n\
             Install it with: pixi global install cosign"
        ));
    }

    let version = String::from_utf8_lossy(&output.stdout);
    tracing::info!("Using cosign version: {}", version.trim());

    Ok(())
}

/// Create an in-toto statement for a conda package
async fn create_intoto_statement(
    package_path: &Path,
    channel_url: &str,
) -> miette::Result<Statement> {
    let package_hash = sha256_sum(package_path).into_diagnostic()?;
    let package_name = package_path
        .file_name()
        .ok_or_else(|| miette::miette!("Package path has no filename"))?
        .to_string_lossy()
        .to_string();

    Ok(Statement {
        statement_type: "https://in-toto.io/Statement/v1".to_string(),
        subject: vec![Subject {
            name: package_name,
            digest: DigestSet {
                sha256: package_hash,
            },
        }],
        predicate_type: "https://schemas.conda.org/attestations-publish-1.schema.json".to_string(),
        predicate: json!({
            "targetChannel": channel_url,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }),
    })
}

/// Sign an in-toto statement using cosign
async fn sign_with_cosign(
    statement: &Statement,
    package_path: &Path,
    config: &AttestationConfig,
) -> miette::Result<String> {
    // Create a temporary file for the statement
    let mut statement_file = NamedTempFile::new().into_diagnostic()?;
    let statement_json = serde_json::to_string_pretty(statement).into_diagnostic()?;

    // Write statement to temp file
    use std::io::Write;
    statement_file
        .write_all(statement_json.as_bytes())
        .into_diagnostic()?;
    statement_file.flush().into_diagnostic()?;

    let statement_path = statement_file.path();

    tracing::debug!(
        "Signing statement with cosign: {}",
        statement_path.display()
    );

    // Build cosign attest command
    // For conda packages, we'll use cosign attest-blob since we don't have an OCI image
    let mut cmd = AsyncCommand::new("cosign");
    cmd.arg("attest-blob")
        .arg("--predicate")
        .arg(statement_path)
        .arg("--output-file")
        .arg("-"); // Output to stdout

    // Configure identity provider
    if config.use_github_oidc {
        // Use GitHub OIDC for identity
        cmd.env("COSIGN_EXPERIMENTAL", "1");

        // Set GitHub-specific environment if available
        if let Ok(server_url) = std::env::var("GITHUB_SERVER_URL") {
            if server_url != "https://github.com" {
                // For GitHub Enterprise, set custom Fulcio URL
                let url = url::Url::parse(&server_url).into_diagnostic()?;
                let host = url
                    .host_str()
                    .ok_or_else(|| miette::miette!("Invalid GitHub server URL"))?;
                cmd.env("SIGSTORE_FULCIO_URL", format!("https://fulcio.{host}"));
                cmd.env("SIGSTORE_REKOR_URL", ""); // GitHub uses TSA, not Rekor
            }
        }
    }

    // Add the blob (package file) to attest
    cmd.arg(package_path);

    tracing::info!("Running cosign to create attestation...");

    // Add timeout to prevent hanging
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
        .await
        .into_diagnostic()
        .map_err(|_| miette::miette!("cosign command timed out after 30 seconds"))?
        .into_diagnostic()
        .map_err(|e| miette::miette!("Failed to run cosign: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(miette::miette!(
            "cosign attestation failed:\n{}\n\nTroubleshooting:\n\
             1. Ensure you're running in GitHub Actions with 'id-token: write' permission\n\
             2. Check that GITHUB_TOKEN is set if uploading to GitHub\n\
             3. For local testing, ensure you have valid credentials configured",
            stderr
        ));
    }

    let bundle_json = String::from_utf8_lossy(&output.stdout).to_string();

    if bundle_json.is_empty() {
        return Err(miette::miette!("cosign produced empty output"));
    }

    tracing::info!("Successfully created attestation with cosign");

    Ok(bundle_json)
}

/// Store a signed attestation bundle to GitHub's attestation API
async fn store_attestation_to_github(
    bundle_json: &str,
    github_token: &str,
    owner: &str,
    repo: &str,
    client: &ClientWithMiddleware,
) -> miette::Result<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/attestations");

    // Parse the bundle JSON to ensure it's valid
    let bundle: serde_json::Value = serde_json::from_str(bundle_json)
        .into_diagnostic()
        .map_err(|e| miette::miette!("Invalid bundle JSON from cosign: {}", e))?;

    let request_body = json!({
        "bundle": bundle,
    });

    tracing::debug!("Storing attestation to GitHub at {}", url);

    let response = client
        .post(&url)
        .bearer_auth(github_token)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&request_body)
        .send()
        .await
        .into_diagnostic()?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.into_diagnostic()?;

        let error_detail = match status.as_u16() {
            401 => "Authentication failed. Check your GitHub token.",
            403 => "Permission denied. Ensure the token has 'attestations:write' permission and the repository allows attestations.",
            404 => "Repository not found or attestations API not available. Ensure you're using a supported GitHub plan.",
            422 => "Invalid attestation bundle format. Check the cosign output.",
            _ => "Unknown error occurred while storing attestation.",
        };

        return Err(miette::miette!(
            "{}\nStatus: {}\nResponse: {}",
            error_detail,
            status,
            body
        ));
    }

    let response_data: AttestationResponse = response.json().await.into_diagnostic()?;
    tracing::info!(
        "Successfully stored attestation with ID: {}",
        response_data.id
    );

    Ok(response_data.id)
}

// Backward compatibility function
pub async fn create_conda_attestation(
    package_path: &Path,
    channel_url: &str,
    _oidc_token: &str,
    client: &ClientWithMiddleware,
) -> miette::Result<serde_json::Value> {
    // Try to extract repo info from environment
    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_else(|_| "unknown/unknown".to_string());
    let parts: Vec<&str> = repo.split('/').collect();
    let (owner, repo_name) = if parts.len() == 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else {
        return Err(miette::miette!(
            "Could not determine repository from GITHUB_REPOSITORY environment variable. \
             Expected format: owner/repo"
        ));
    };

    let github_token = std::env::var("GITHUB_TOKEN")
        .into_diagnostic()
        .map_err(|_| miette::miette!("GITHUB_TOKEN environment variable not set"))?;

    let config = AttestationConfig {
        repo_owner: owner,
        repo_name,
        github_token,
        use_github_oidc: true,
    };

    let _attestation_id =
        create_attestation_with_cosign(package_path, channel_url, &config, client).await?;

    // Return a simple success indicator for backward compatibility
    Ok(json!({
        "success": true,
        "message": "Attestation created with cosign"
    }))
}
