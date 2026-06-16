use clap::Parser;
use std::{collections::HashMap, error::Error};

type JobFunction = Box<dyn Fn(&str) -> Result<(), Box<dyn Error + Send + Sync>>>;

#[derive(Parser)]
#[command(about = "Run a registered experiment job")]
struct Args {
    /// Job name to run (uses default if omitted)
    job: Option<String>,
    /// Config string passed to the job's mapper
    config: Option<String>,
}

pub struct Cli {
    jobs: HashMap<String, JobFunction>,
    default: Option<String>,
}

impl Cli {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            default: None,
        }
    }
}
