//! Native Sigstore attestation creation for conda packages
//!
//! This module provides attestation creation using the sigstore-sign crate.

use miette::{IntoDiagnostic, WrapErr};
use reqwest::header;
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sigstore_sign::{Attestation, SigningContext, SigningServices, oidc::IdentityToken};
use sigstore_trust_root::SigningConfig as TufSigningConfig;
use std::path::Path;

/// Conda V1 predicate
#[derive(Debug, Serialize, Deserialize)]
pub struct CondaV1Predicate {
    #[serde(rename = "targetChannel", skip_serializing_if = "Option::is_none")]
    pub target_channel: Option<String>,
}

/// Response from GitHub attestation API
#[derive(Debug, Serialize, Deserialize)]
pub struct AttestationResponse {
    pub id: u64,
}

/// Configuration for attestation creation
#[derive(Debug, Clone)]
pub struct AttestationConfig {
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub github_token: Option<String>,
}

/// The predicate type of conda publish attestations (CEP-27).
pub const CONDA_PUBLISH_PREDICATE_TYPE: &str =
    "https://schemas.conda.org/attestations-publish-1.schema.json";

/// The package an attestation is about: its filename and SHA-256 digest.
///
/// Signing only needs these two values, so a package can be attested without
/// having the file at hand (e.g. in a CI job separate from the build).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttestationSubject {
    /// The package filename, e.g. `foo-1.0-h123_0.conda`.
    pub filename: String,
    /// The SHA-256 digest of the package file.
    pub sha256: rattler_digest::Sha256Hash,
}

impl AttestationSubject {
    /// Create a subject from a package filename and its SHA-256 digest.
    pub fn new(filename: impl Into<String>, sha256: rattler_digest::Sha256Hash) -> Self {
        Self {
            filename: filename.into(),
            sha256,
        }
    }

    /// Create a subject from a package file, computing its SHA-256 digest.
    pub fn from_path(package_path: &Path) -> miette::Result<Self> {
        let filename = package_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| miette::miette!("Invalid package file name"))?;
        let sha256 = rattler_digest::compute_file_digest::<rattler_digest::Sha256>(package_path)
            .into_diagnostic()
            .with_context(|| format!("failed to hash {}", package_path.display()))?;
        Ok(Self::new(filename, sha256))
    }
}

/// Signs CEP-27 publish attestations with Sigstore.
///
/// The Sigstore signing configuration is fetched once, so one attester can
/// sign attestations for many packages. Each signature uses a fresh OIDC
/// identity token from the environment (GitHub Actions, GitLab CI, ...).
#[derive(Debug, Clone)]
pub struct Attester {
    context: SigningContext,
}

impl Attester {
    /// Create an attester for the production Sigstore instance, fetching its
    /// signing configuration via TUF.
    pub async fn production() -> miette::Result<Self> {
        tracing::info!("Fetching Sigstore signing config via TUF...");
        let tuf_config = TufSigningConfig::production()
            .await
            .map_err(|e| miette::miette!("Failed to fetch Sigstore TUF signing config: {}", e))?;
        let signing_services = SigningServices::from_signing_config(&tuf_config)
            .map_err(|e| miette::miette!("Failed to build Sigstore signing config: {}", e))?;
        Ok(Self {
            context: SigningContext::new(signing_services),
        })
    }

    /// Sign a publish attestation for one package and return the Sigstore
    /// bundle as JSON. `channel_url` is recorded as `targetChannel`.
    pub async fn attest(
        &self,
        subject: &AttestationSubject,
        channel_url: &str,
    ) -> miette::Result<String> {
        let identity_token = detect_identity_token().await?;

        let predicate = CondaV1Predicate {
            target_channel: Some(channel_url.to_string()),
        };
        let sha256_hash = sigstore_sign::types::Sha256Hash::from_hex(&hex::encode(subject.sha256))
            .map_err(|e| miette::miette!("Invalid SHA256 hash: {}", e))?;
        let attestation = Attestation::new(
            CONDA_PUBLISH_PREDICATE_TYPE,
            serde_json::to_value(&predicate).into_diagnostic()?,
        )
        .add_subject(&subject.filename, sha256_hash);

        tracing::info!(
            "Signing attestation for {} with Sigstore...",
            subject.filename
        );
        let bundle = self
            .context
            .signer(identity_token)
            .sign_attestation(attestation)
            .await
            .map_err(|e| miette::miette!("Failed to sign attestation with Sigstore: {}", e))?;

        bundle
            .to_json_pretty()
            .map_err(|e| miette::miette!("Failed to serialize bundle: {}", e))
    }

    /// Sign a publish attestation for one package and, if `config` has a
    /// GitHub repository and token, store it in GitHub's attestation API.
    /// Returns the Sigstore bundle as JSON.
    pub async fn attest_and_store(
        &self,
        subject: &AttestationSubject,
        channel_url: &str,
        config: &AttestationConfig,
        client: &ClientWithMiddleware,
    ) -> miette::Result<String> {
        let bundle_json = self.attest(subject, channel_url).await?;
        if let (Some(token), Some(owner), Some(repo)) =
            (&config.github_token, &config.repo_owner, &config.repo_name)
        {
            let attestation_id =
                store_attestation_to_github(&bundle_json, token, owner, repo, client).await?;
            tracing::info!("Attestation stored to GitHub with ID: {}", attestation_id);
        }
        Ok(bundle_json)
    }
}

async fn detect_identity_token() -> miette::Result<IdentityToken> {
    IdentityToken::detect_ambient()
        .await
        .into_diagnostic()
        .with_context(|| "Failed to detect OIDC identity token")?
        .ok_or_else(|| miette::miette!("No OIDC identity token found in environment. Attestation signing is supported in GitHub Actions, GitLab CI, and other OIDC-enabled CI environments."))
}

/// Create and store an attestation for a conda package using native Sigstore signing
///
/// This function:
/// 1. Creates an in-toto statement for the package
/// 2. Uses Sigstore (Fulcio + Rekor) to sign the statement with OIDC identity
/// 3. Optionally stores the signed attestation to GitHub's attestation API (if token provided)
///
/// Returns the attestation bundle JSON. To attest several packages, or a
/// package that is not available locally, use [`Attester`].
pub async fn create_attestation(
    package_path: &Path,
    channel_url: &str,
    config: &AttestationConfig,
    client: &ClientWithMiddleware,
) -> miette::Result<String> {
    // Fail early, before fetching the signing config, when not in CI.
    detect_identity_token().await?;
    let subject = AttestationSubject::from_path(package_path)?;
    Attester::production()
        .await?
        .attest_and_store(&subject, channel_url, config, client)
        .await
}

/// Store a signed attestation bundle to GitHub's attestation API.
///
/// Returns the ID of the stored attestation.
pub async fn store_attestation_to_github(
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
        .map_err(|e| miette::miette!("Invalid bundle JSON: {}", e))?;

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

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.into_diagnostic()?;
        let error_detail = match status.as_u16() {
            401 => "Authentication failed. Check your GitHub token.",
            403 => {
                "Permission denied. Ensure token has 'attestations:write' and repository allows attestations."
            }
            404 => {
                "Repository not found or attestations API unavailable. Ensure you're on a supported GitHub plan."
            }
            422 => "Invalid attestation bundle format.",
            _ => "Unknown error storing attestation.",
        };

        return Err(miette::miette!(
            "{}\nStatus: {}\nResponse: {}",
            error_detail,
            status,
            body
        ));
    }

    let body = response.text().await.into_diagnostic()?;
    let response_data: AttestationResponse = serde_json::from_str(&body)
        .map_err(|e| miette::miette!("Failed to parse GitHub response: {}\nBody: {}", e, body))?;
    tracing::info!(
        "Successfully stored attestation with ID: {}",
        response_data.id
    );

    Ok(response_data.id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_from_path_hashes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo-1.0-h123_0.conda");
        fs_err::write(&path, b"x").unwrap();

        let subject = AttestationSubject::from_path(&path).unwrap();
        assert_eq!(subject.filename, "foo-1.0-h123_0.conda");
        assert_eq!(
            hex::encode(subject.sha256),
            "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
        );
    }
}
