use std::io::BufReader;

#[derive(serde::Deserialize)]
pub struct BUUU {
    pub name: String,
}

pub fn runner_main() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let buf = BufReader::new(stdin);

    let payload: BUUU = serde_json::from_reader(buf)?;

    Ok(())
}
