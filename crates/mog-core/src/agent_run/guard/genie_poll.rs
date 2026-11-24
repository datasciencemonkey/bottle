//! The Rust poll loop of one Genie call (ADR 0083, Section 2.1 of the slice 2 plan).
//!
//! It is the Rust copy of the brain PollActor (`src/brain/components/poll-actor.ts`): the same
//! Table G-POLL delays, the G-08 floor, the 30 s cap of one poll (ARC-F4) and the 10 min give-up
//! (G-12b). The loop never calls `genie_ask`, so it cannot ask a question twice (G-13b). It holds
//! no answer text: the reply of the last poll goes back to the caller.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use super::Sleeper;
use crate::clock::MonotonicClock;
use crate::ipc::JsonObject;
use crate::mcp_transport::agent_asks::{AgentAsks, GIVE_UP_AFTER, TERMINAL_STATUSES};
use crate::mcp_transport::{McpCall, McpReply, McpTransport, E_POLL_INFLIGHT};
use crate::types::Tags;

/// ARC-F4: the time cap of one poll.
pub const POLL_TIMEOUT: Duration = Duration::from_secs(30);
/// G-08: the delay floor for each in-flight ask on the profile.
pub const INFLIGHT_FLOOR: Duration = Duration::from_millis(1_500);
/// Table G-POLL: the longest base delay.
pub const MAX_POLL_DELAY: Duration = Duration::from_secs(15);
/// Table G-POLL: the jitter on every step (+-10 %).
pub const POLL_JITTER: f64 = 0.1;

/// The base delay of Table G-POLL for an ask that is `elapsed` old. After 180 s the base is the
/// previous base x 1.5, at most 15 s.
pub fn base_poll_delay(elapsed: Duration, previous_base: Option<Duration>) -> Duration {
    let s = elapsed.as_secs_f64();
    if s < 20.0 {
        Duration::from_secs(2)
    } else if s < 60.0 {
        Duration::from_secs(3)
    } else if s < 180.0 {
        Duration::from_secs(5)
    } else {
        previous_base
            .unwrap_or(Duration::from_secs(5))
            .max(Duration::from_secs(5))
            .mul_f64(1.5)
            .min(MAX_POLL_DELAY)
    }
}

/// The delay before the next poll (G-07c, G-08): the base, +-10 % jitter, at least 1.5 s x the
/// in-flight asks on the profile. `random` is in [0, 1).
pub fn poll_delay(
    elapsed: Duration,
    previous_base: Option<Duration>,
    inflight: usize,
    random: f64,
) -> (Duration, Duration) {
    let base = base_poll_delay(elapsed, previous_base);
    let r = random.clamp(0.0, 1.0);
    let jittered = base.mul_f64(1.0 + POLL_JITTER * (2.0 * r - 1.0));
    let floor = INFLIGHT_FLOOR * u32::try_from(inflight.max(1)).unwrap_or(u32::MAX);
    (base, jittered.max(floor))
}

/// What a poll loop needs. It holds no content.
pub struct PollEnv<'a> {
    pub transport: &'a dyn McpTransport,
    pub clock: &'a dyn MonotonicClock,
    pub sleeper: &'a dyn Sleeper,
    pub random: &'a (dyn Fn() -> f64 + Send + Sync),
    pub asks: &'a AgentAsks,
    pub tags: &'a Tags,
    pub profile: &'a str,
    /// The in-flight calls on `profile`, for the G-08 floor.
    pub inflight: &'a (dyn Fn() -> usize + Send + Sync),
    /// Turns true when the call must stop (a user stop, the end of the session).
    pub cancel: &'a AtomicBool,
}

/// How a poll loop ends.
#[derive(Debug)]
pub enum PollEnd {
    /// A terminal status, with the reply of that poll.
    Terminal(McpReply),
    /// The give-up time came (G-12b).
    GaveUp,
    /// `cancel` turned true.
    Cancelled,
    /// A poll failed with this code. Nothing else is kept.
    Failed(&'static str),
}

/// True if `sc` has a terminal status.
pub fn is_terminal(sc: &Value) -> bool {
    sc.get("status")
        .and_then(Value::as_str)
        .is_some_and(|s| TERMINAL_STATUSES.contains(&s))
}

fn poll_args(conversation: &str, response: &str) -> JsonObject {
    let mut a = JsonObject::new();
    a.insert("conversation_id".into(), conversation.into());
    a.insert("response_id".into(), response.into());
    a
}

/// Poll `conversation` until a terminal status, the give-up or a cancel. `asked_at` is the time
/// of the ask on `env.clock`.
pub fn run(env: &PollEnv<'_>, conversation: &str, response: &str, asked_at: Duration) -> PollEnd {
    let mut previous_base: Option<Duration> = None;
    loop {
        if env.cancel.load(Ordering::SeqCst) {
            return PollEnd::Cancelled;
        }
        let elapsed = env.clock.now().saturating_sub(asked_at);
        let (base, delay) = poll_delay(elapsed, previous_base, (env.inflight)(), (env.random)());
        previous_base = Some(base);
        // G-08: an ask that starts while this loop sleeps raises the floor too, so check it
        // again on each wake-up and sleep on to the later time.
        let from = env.clock.now();
        let mut due = from + delay;
        loop {
            // ARC-F4, G-12b: no poll starts after the give-up.
            if due.saturating_sub(asked_at) > GIVE_UP_AFTER {
                return PollEnd::GaveUp;
            }
            env.sleeper
                .sleep(due.saturating_sub(env.clock.now()), env.cancel);
            if env.cancel.load(Ordering::SeqCst) {
                return PollEnd::Cancelled;
            }
            let floor_due =
                from + INFLIGHT_FLOOR * u32::try_from((env.inflight)().max(1)).unwrap_or(1);
            if floor_due <= env.clock.now() {
                break;
            }
            due = floor_due;
        }
        if env.asks.may_poll(conversation).is_err() {
            return PollEnd::GaveUp;
        }
        let call = McpCall::new(
            env.profile,
            "genie_poll_response",
            poll_args(conversation, response),
            env.tags,
        )
        .timeout(POLL_TIMEOUT);
        let reply = match env.transport.call_tool(call) {
            Ok(r) => r,
            Err(e) => match e.code() {
                // G-16c: a poll timeout polls again with the same IDs. ARC-F1: another poll is
                // open on this conversation. ADR 0075: the bucket refused the poll.
                "timeout" | "rate_limited" => continue,
                c if c == E_POLL_INFLIGHT => continue,
                c => return PollEnd::Failed(c),
            },
        };
        if reply.is_error {
            return PollEnd::Failed("genie_error");
        }
        env.asks.polled(conversation, &reply.structured_content);
        if is_terminal(&reply.structured_content) {
            return PollEnd::Terminal(reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn g_poll_base_delays_follow_the_table() {
        assert_eq!(base_poll_delay(s(0), None), s(2));
        assert_eq!(base_poll_delay(s(19), None), s(2));
        assert_eq!(base_poll_delay(s(20), None), s(3));
        assert_eq!(base_poll_delay(s(59), None), s(3));
        assert_eq!(base_poll_delay(s(60), None), s(5));
        assert_eq!(base_poll_delay(s(179), None), s(5));
        // After 180 s: the previous base x 1.5, at most 15 s.
        assert_eq!(base_poll_delay(s(180), None), Duration::from_millis(7_500));
        assert_eq!(
            base_poll_delay(s(200), Some(Duration::from_millis(7_500))),
            Duration::from_millis(11_250)
        );
        assert_eq!(base_poll_delay(s(300), Some(s(11))), s(15));
        assert_eq!(base_poll_delay(s(900), Some(s(15))), s(15));
    }

    #[test]
    fn g_07c_g_08_jitter_is_ten_percent_and_the_floor_scales_with_inflight_asks() {
        assert_eq!(poll_delay(s(0), None, 1, 0.5).1, s(2));
        assert_eq!(
            poll_delay(s(0), None, 1, 0.0).1,
            Duration::from_millis(1_800)
        );
        assert_eq!(
            poll_delay(s(0), None, 1, 1.0).1,
            Duration::from_millis(2_200)
        );
        // 3 asks in flight: the floor is 4.5 s.
        assert_eq!(
            poll_delay(s(0), None, 3, 0.5).1,
            Duration::from_millis(4_500)
        );
        // No in-flight ask counts as 1.
        assert_eq!(poll_delay(s(0), None, 0, 0.5).1, s(2));
    }
}
