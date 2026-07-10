pub mod append;
pub mod auth;
pub mod copy;
pub mod expunge;
pub mod fetch;
pub mod idle;
pub mod mailbox;
pub mod noop;
pub mod search;
pub mod select;
pub mod store;
pub mod writer;

use tracing::{debug, error};

use imap_core::codec::ParsedCommand;
use imap_core::error::ImapError;
use imap_core::response::ResponseCode;
use imap_core::types::ConnState;

use crate::conn::Conn;

/// Dispatch a parsed command to the appropriate handler.
/// Returns Err(Closed) only when the connection should be torn down.
/// All other errors are caught and written as tagged NO/BAD responses.
pub async fn dispatch(conn: &mut Conn, cmd: ParsedCommand) -> Result<(), ImapError> {
    let ParsedCommand { tag, verb, uid } = cmd;
    debug!("cmd: tag={tag} verb={verb} uid={uid}");

    let result = match verb.as_str() {
        "NOOP" => noop::handle_noop(conn, &tag).await,
        "CHECK" => noop::handle_check(conn, &tag).await,
        "LOGOUT" => auth::handle_logout(conn, &tag).await,
        "CAPABILITY" => auth::handle_capability(conn, &tag).await,
        "ID" => auth::handle_id(conn, &tag).await,
        "STARTTLS" => auth::handle_starttls(conn, &tag).await,
        "LOGIN" => auth::handle_login(conn, &tag).await,
        "AUTHENTICATE" => auth::handle_authenticate(conn, &tag).await,
        "UNAUTHENTICATE" => auth::handle_unauthenticate(conn, &tag).await,
        "ENABLE" => noop::handle_enable(conn, &tag).await,
        "SELECT" => select::handle_select(conn, &tag, false).await,
        "EXAMINE" => select::handle_select(conn, &tag, true).await,
        "CLOSE" => select::handle_close(conn, &tag).await,
        "UNSELECT" => select::handle_unselect(conn, &tag).await,
        "CREATE" => mailbox::handle_create(conn, &tag).await,
        "DELETE" => mailbox::handle_delete(conn, &tag).await,
        "RENAME" => mailbox::handle_rename(conn, &tag).await,
        "SUBSCRIBE" => mailbox::handle_subscribe(conn, &tag).await,
        "UNSUBSCRIBE" => mailbox::handle_unsubscribe(conn, &tag).await,
        "LIST" => mailbox::handle_list(conn, &tag).await,
        "LSUB" => mailbox::handle_lsub(conn, &tag).await,
        "STATUS" => mailbox::handle_status(conn, &tag).await,
        "APPEND" => append::handle_append(conn, &tag).await,
        "FETCH" => fetch::handle_fetch(conn, &tag, uid).await,
        "STORE" => store::handle_store(conn, &tag, uid).await,
        "SEARCH" => search::handle_search(conn, &tag, uid).await,
        "COPY" => copy::handle_copy(conn, &tag, uid).await,
        "MOVE" => copy::handle_move(conn, &tag, uid).await,
        "EXPUNGE" => expunge::handle_expunge(conn, &tag).await,
        "UID EXPUNGE" => expunge::handle_uid_expunge(conn, &tag).await,
        "IDLE" => idle::handle_idle(conn, &tag).await,
        "NAMESPACE" => noop::handle_namespace(conn, &tag).await,
        _ => {
            // Cross-protocol attack mitigation: if in NotAuthenticated state
            // and the command is unknown, send BYE and disconnect.
            if conn.state == ConnState::NotAuthenticated {
                conn.write_bye("Unknown command in not-authenticated state").await;
                return Err(ImapError::Closed);
            }
            Err(ImapError::bad_code(
                ResponseCode::CLIENTBUG,
                format!("Unknown command: {verb}"),
            ))
        }
    };

    // Handle the result: write tagged error response or poll.
    // Handlers send their own tagged OK on success — dispatch doesn't duplicate.
    match result {
        Ok(()) => {
            // Poll for changes (skip after LOGOUT/STARTTLS)
            if !matches!(verb.as_str(), "LOGOUT" | "STARTTLS") {
                if let Some(session) = &mut conn.session {
                    let _ = session.poll().await;
                }
            }
            Ok(())
        }
        Err(ImapError::Closed) => Err(ImapError::Closed),
        Err(ImapError::No { code, text }) => {
            let code_str = code.as_deref();
            conn.write_status(&tag, "NO", code_str, &text).await;
            Ok(())
        }
        Err(ImapError::Bad { code, text }) => {
            let code_str = code.as_deref();
            conn.write_status(&tag, "BAD", code_str, &text).await;
            Ok(())
        }
        Err(ImapError::Internal(e)) => {
            error!("internal error handling {verb}: {e}");
            conn.write_status(
                &tag,
                "NO",
                Some(ResponseCode::SERVERBUG),
                "Internal server error",
            )
            .await;
            Ok(())
        }
    }
}
