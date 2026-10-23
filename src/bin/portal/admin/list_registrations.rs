use status_list_portal::ServerContext;
use status_list_portal::models::Registration;
use time::format_description::well_known::Rfc3339;

pub async fn run(ctx: &ServerContext) -> anyhow::Result<()> {
    let mut conn = ctx.db.get().await?;
    let pending = Registration::pending(&mut conn).await?;
    if pending.is_empty() {
        println!("No registration waits for approval.");
        return Ok(());
    }

    println!("ID\tASKED AT\tACCOUNT\tEMAIL\tALG\tDISPLAY NAME");
    for (registration, user) in pending {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            registration.id,
            registration.created_at.format(&Rfc3339)?,
            user.gh_login,
            user.email.escape_debug(),
            registration.alg,
            registration.display_name.escape_debug(),
        );
    }
    Ok(())
}
