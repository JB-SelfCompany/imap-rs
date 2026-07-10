use std::time::Duration;

use imap_core::error::{ImapError, ImapResult};

use crate::conn::Conn;

/// Handle IDLE command (RFC 2177).
///
/// Correct flow:
/// 1. Validate selected state and consume CRLF
/// 2. Send continuation ("+ idling")
/// 3. Enter select loop waiting for DONE or periodic poll ticks
/// 4. On each poll tick, call `session.poll()` so the backend can drain
///    tracker updates and write untagged responses
/// 5. On DONE, break and send tagged OK
pub async fn handle_idle(conn: &mut Conn, tag: &str) -> ImapResult<()> {
    conn.require_selected()?;
    conn.decoder.expect_crlf().await?;

    // Send continuation FIRST (RFC 2177)
    conn.write_continuation("idling")
        .await
        .map_err(|e| ImapError::Internal(Box::new(e)))?;

    // Poll interval: check for tracker updates every second
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    // The first tick fires immediately; skip it
    interval.tick().await;

    // Split borrows so the select! macro can access decoder and session
    // independently (they are disjoint fields of Conn).
    {
        let decoder = &mut conn.decoder;
        let encoder = &mut conn.encoder;
        let session = conn
            .session
            .as_mut()
            .ok_or_else(|| ImapError::bad("No session"))?;

        // Baseline message count at IDLE entry; only later *increases* are
        // reported as EXISTS during this IDLE.
        let mut last_count = session.current_message_count().await;

        loop {
            tokio::select! {
                // Biased: always check for DONE first so the client can
                // terminate IDLE promptly even under high update traffic.
                biased;

                result = decoder.read_atom() => {
                    match result {
                        Ok(s) if s.eq_ignore_ascii_case("DONE") => {
                            decoder.expect_crlf().await?;
                            break;
                        }
                        Ok(_) => {
                            // Not DONE — discard the rest of the line and
                            // keep waiting.
                            decoder.discard_line().await;
                        }
                        Err(e) => return Err(e),
                    }
                }

                _ = interval.tick() => {
                    // Let the backend drain any tracker state first.
                    let _ = session.poll().await;
                    // Then push an untagged EXISTS (RFC 2177) when the selected
                    // mailbox has grown, so IDLE clients (e.g. DeltaChat) see new
                    // mail without reconnecting. Previously nothing woke the IDLE
                    // client, so YMP-delivered mail sat invisible until a restart.
                    if let Some(count) = session.current_message_count().await {
                        if count > last_count.unwrap_or(0) {
                            encoder
                                .atom("*").await
                                .sp().await
                                .number(count).await
                                .sp().await
                                .atom("EXISTS").await
                                .crlf().await
                                .map_err(|e| ImapError::Internal(Box::new(e)))?;
                        }
                        last_count = Some(count);
                    }
                }
            }
        }
    }
    // Borrows released here; conn is fully owned again.

    conn.write_ok(tag, "IDLE terminated").await;
    Ok(())
}
