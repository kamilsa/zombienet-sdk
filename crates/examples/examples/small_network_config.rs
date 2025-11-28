use zombienet_sdk::{NetworkConfigBuilder, NetworkConfigExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = NetworkConfigBuilder::new()
        .with_relaychain(|r| {
            r.with_chain("rococo-local")
                .with_validator(|node| node.with_name("alice").with_command("polkadot"))
        })
        .build()
        .map_err(|e| anyhow::anyhow!("Config build errors: {:?}", e))?;

    let _network = config.spawn_shadow().await?;

    println!("Shadow network spawned!");

    Ok(())
}
