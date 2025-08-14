use clap::{Args, Parser};

#[derive(Args, Debug)]
pub struct BurnCentralArgs {
    #[arg(long, default_value = "default")]
    pub namespace: String,
    #[arg(long, default_value = "default")]
    pub project: String,
    #[arg(long)]
    pub api_key: String,
    #[arg(long, default_value = "http://localhost:9001")]
    pub endpoint: String,
}

#[derive(Parser, Debug)]
#[command(
    name = "burn-central-runtime",
    version,
    about = "Burn Central Runtime CLI"
)]
pub struct RuntimeArgs {
    pub kind: String,
    pub routine: String,
    #[arg(long, default_value = "{}")]
    pub config: String,
    #[command(flatten)]
    pub burn_central: BurnCentralArgs,
}

pub fn parse_runtime_args() -> RuntimeArgs {
    RuntimeArgs::parse()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_runtime_args() {
        let args = vec![
            "burn-central-runtime",
            "train",
            "my_routine",
            "--namespace",
            "my_namespace",
            "--project",
            "my_project",
            "--api-key",
            "my_api_key",
        ];
        let runtime_args = RuntimeArgs::try_parse_from(args).unwrap();
        assert_eq!(runtime_args.kind, "train");
        assert_eq!(runtime_args.routine, "my_routine");
        assert_eq!(runtime_args.config, "{}");
        assert_eq!(runtime_args.burn_central.namespace, "my_namespace");
        assert_eq!(runtime_args.burn_central.project, "my_project");
        assert_eq!(runtime_args.burn_central.api_key, "my_api_key");
    }
}
