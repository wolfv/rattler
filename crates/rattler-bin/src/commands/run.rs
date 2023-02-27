use rattler_shell::run::run_in_prefix;

#[derive(Debug, clap::Parser)]
pub struct Opt {
    #[clap(required = true)]
    cmd: Vec<String>,
}

pub async fn run(opt: Opt) -> anyhow::Result<()> {
    let prefix = std::env::current_dir()?.join(".prefix");
    run_in_prefix(opt.cmd, prefix)?;
    Ok(())
}