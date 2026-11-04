use anyhow::{anyhow, bail};
use status_list_portal::{ServerContext, sandbox};

/// The longest reason a developer is shown.
const REASON_MAX_CHARS: usize = 500;

/// Rejects a registration. The developer sees the reason.
#[derive(Debug, clap::Args)]
pub struct Opts {
    /// The registration's id, from `list-registrations`.
    id: i64,
    /// Why, in words for the developer. At most 500 characters, on one line.
    #[arg(long)]
    reason: Option<String>,
}

pub async fn run(ctx: &ServerContext, opts: Opts) -> anyhow::Result<()> {
    if let Some(reason) = &opts.reason
        && (reason.chars().count() > REASON_MAX_CHARS || reason.contains(char::is_control))
    {
        bail!("The reason must be at most {REASON_MAX_CHARS} characters, on one line.");
    }

    let mut conn = ctx.db.get().await?;
    let id = opts.id;
    sandbox::reject(&mut conn, id, opts.reason.as_deref())
        .await
        .map_err(|error| anyhow!("{error}"))?;
    println!("Rejected registration {id}.");
    Ok(())
}
