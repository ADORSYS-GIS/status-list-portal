use anyhow::anyhow;
use status_list_portal::{ServerContext, sandbox};

/// Rejects a registration. The developer sees the reason.
#[derive(Debug, clap::Args)]
pub struct Opts {
    /// The registration's id, from `list-registrations`.
    id: i64,
    /// Why, in words for the developer. At most 500 characters, on one line.
    #[arg(long)]
    reason: String,
}

pub async fn run(ctx: &ServerContext, opts: Opts) -> anyhow::Result<()> {
    let mut conn = ctx.db.get().await?;
    let id = opts.id;
    sandbox::reject(&mut conn, id, &opts.reason)
        .await
        .map_err(|error| anyhow!("{error}"))?;
    println!("Rejected registration {id}.");
    Ok(())
}
