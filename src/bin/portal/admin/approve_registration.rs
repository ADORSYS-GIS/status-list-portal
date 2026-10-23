use anyhow::anyhow;
use status_list_portal::{ServerContext, sandbox};
use time::format_description::well_known::Rfc3339;

/// Approves a registration: creates its issuer, if the sandbox limits leave room for it.
#[derive(Debug, clap::Args)]
pub struct Opts {
    /// The registration's id, from `list-registrations`.
    id: i64,
}

pub async fn run(ctx: &ServerContext, opts: Opts) -> anyhow::Result<()> {
    let mut conn = ctx.db.get().await?;
    let issuer = sandbox::approve(ctx, &mut conn, opts.id)
        .await
        .map_err(|error| anyhow!("{error}"))?;
    let until = match issuer.expires_at {
        Some(expires_at) => expires_at.format(&Rfc3339)?,
        None => "it is suspended".to_string(),
    };
    println!(
        "Approved registration {}: issuer {} is active until {until}.",
        opts.id, issuer.id
    );
    Ok(())
}
