//! Requests to the AI service that are tried again by themselves (#2037).
//! A service that is busy for a moment (503 and its like, or a rate
//! limit that asks for a short wait) answered the user with an error at
//! once, and the only way on was to press Draw again or try another
//! model. A [`Call`] sends its request again after a wait that grows
//! (2, 4, 8 seconds, or what the service's `Retry-After` asks when that
//! is short), up to [`MAX_TRIES`] times in all, and only then hands the
//! error on. The waiting happens between frames ([`Call::poll`] is asked
//! each frame, as the reply always was), so it works the same on the
//! desktop and in a browser, and dropping the call cancels it.
//! See docs/behavior/ai-drawing.md.

use crate::net::{HttpRequest, HttpResponse, HttpResult};
use std::time::Duration;

/// How often a request is sent in all: once, then three more tries.
pub const MAX_TRIES: u32 = 4;
/// The wait before the second try; it doubles for each one after.
const FIRST_WAIT: Duration = Duration::from_secs(2);
/// The longest wait a service may ask for with `Retry-After` and still
/// have the request sent again by itself; a longer one is shown to the
/// user instead.
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// A status that says the service is busy or down for the moment: an
/// internal error (500), a gateway without its server (502), unavailable
/// or overloaded (503, 529).
pub fn busy_status(status: u16) -> bool {
    matches!(status, 500 | 502 | 503 | 529)
}

/// The seconds a `Retry-After` header asks for, when it gives seconds
/// (a date is left to the message shown).
fn retry_after_secs(r: &HttpResponse) -> Option<u64> {
    r.retry_after.as_deref()?.trim().parse::<u64>().ok()
}

/// How long to wait before sending the request again after `result`,
/// the answer to try number `tries` (1 for the first); `None` when it is
/// not sent again: an answer that is no passing trouble, the last try, or
/// a service that asks for a longer wait than [`LONGEST_WAIT`].
///
/// - 500, 502, 503 and 529: the service's `Retry-After` when it gives
///   one, else 2 s after the first try, 4 s after the second, 8 s after
///   the third.
/// - 429 (too many requests, or a used up quota): only when the service
///   says how long to wait; without that it is most often a quota that a
///   wait of seconds does not bring back.
/// - Everything else (a refused key, an unknown model, a reply that
///   came, a network failure): never.
pub fn wait_before_retry(tries: u32, result: &HttpResult) -> Option<Duration> {
    let Ok(r) = result else {
        return None;
    };
    if tries == 0 || tries >= MAX_TRIES {
        return None;
    }
    let asked = retry_after_secs(r).map(Duration::from_secs);
    let wait = match r.status {
        s if busy_status(s) => asked.unwrap_or(FIRST_WAIT * 2u32.pow(tries - 1)),
        429 => asked?,
        _ => return None,
    };
    (wait <= LONGEST_WAIT).then_some(wait)
}

/// Tests: answers requests at once instead of the network.
pub type Fake = fn(&HttpRequest) -> HttpResult;

/// Where a call stands.
enum State {
    /// Sent; the answer comes on the channel.
    Sent(std::sync::mpsc::Receiver<HttpResult>),
    /// The service was busy: sent again at this time. The status it
    /// answered is kept for the note shown meanwhile.
    Waiting {
        until: web_time::Instant,
        status: u16,
    },
}

/// One request and its further tries.
pub struct Call {
    http: HttpRequest,
    /// The most bytes of answer read.
    limit: u64,
    state: State,
    /// Tries sent so far.
    tries: u32,
}

/// What [`Call::poll`] found.
pub enum Polled {
    /// No answer yet, or waiting to try again.
    Pending,
    /// The request ended without an answer or an error (its thread is
    /// gone).
    Lost,
    /// The answer that stands (or why none came), after this many tries.
    Done(HttpResult, u32),
}

/// The try a call waits to make: `(status answered, try, of)`.
pub type Retrying = (u16, u32, u32);

impl Call {
    /// Send `http` (through `fake` in tests).
    pub fn start(http: HttpRequest, limit: u64, fake: Option<Fake>) -> Call {
        let state = State::Sent(send(&http, limit, fake));
        Call {
            http,
            limit,
            state,
            tries: 1,
        }
    }

    /// The request, as sent.
    #[cfg(test)]
    pub fn request(&self) -> &HttpRequest {
        &self.http
    }

    /// The try the call waits to make, while the service was busy.
    pub fn retrying(&self) -> Option<Retrying> {
        match self.state {
            State::Waiting { status, .. } => Some((status, self.tries + 1, MAX_TRIES)),
            State::Sent(_) => None,
        }
    }

    /// Look for the answer; send the request again when its wait is
    /// over. `fake` answers in tests, where `fake_wait` stands for every
    /// wait. A request sent again is pushed on `sent` (tests look there).
    pub fn poll(
        &mut self,
        fake: Option<Fake>,
        fake_wait: Duration,
        sent: &mut Vec<HttpRequest>,
    ) -> Polled {
        // With a test transport every step happens at once, so the loop
        // runs a call to its end; on the network each turn leaves it.
        loop {
            let next = match &self.state {
                State::Sent(rx) => {
                    let result = match rx.try_recv() {
                        Ok(r) => r,
                        Err(std::sync::mpsc::TryRecvError::Empty) => return Polled::Pending,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return Polled::Lost,
                    };
                    let Some(wait) = wait_before_retry(self.tries, &result) else {
                        return Polled::Done(result, self.tries);
                    };
                    let wait = if fake.is_some() { fake_wait } else { wait };
                    State::Waiting {
                        until: web_time::Instant::now() + wait,
                        status: result.map(|r| r.status).unwrap_or_default(),
                    }
                }
                State::Waiting { until, .. } => {
                    if web_time::Instant::now() < *until {
                        return Polled::Pending;
                    }
                    self.tries += 1;
                    sent.push(self.http.clone());
                    State::Sent(send(&self.http, self.limit, fake))
                }
            };
            self.state = next;
        }
    }
}

/// Send `http`: the answer comes on the channel returned.
fn send(
    http: &HttpRequest,
    limit: u64,
    fake: Option<Fake>,
) -> std::sync::mpsc::Receiver<HttpResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    match fake {
        Some(fake) => {
            let _ = tx.send(fake(http));
        }
        None => crate::net::spawn(http.clone(), limit, tx),
    }
    rx
}

/// The note shown while a call waits to try again.
pub fn note((status, n, max): Retrying) -> String {
    crate::i18n::trf(
        "ai.busy_retrying",
        &[
            ("status", &status.to_string()),
            ("n", &n.to_string()),
            ("max", &max.to_string()),
        ],
    )
}

/// The message for an answer that was still "busy" after every try:
/// said as such, with what to do, and the service's own words. `None`
/// for any other answer (the usual messages say those).
pub fn gave_up(r: &HttpResponse, tries: u32) -> Option<String> {
    // A rate limit (429) that outlasted the tries keeps its usual
    // message, which says what it is.
    if tries < 2 || !busy_status(r.status) {
        return None;
    }
    let mut out = crate::i18n::trf(
        "ai.error_busy_retried",
        &[("status", &r.status.to_string()), ("n", &tries.to_string())],
    );
    let v: Option<serde_json::Value> = serde_json::from_str(&r.body).ok();
    let said = crate::ai::service_words(r, v.as_ref());
    if !said.is_empty() {
        out.push(' ');
        out.push_str(&crate::i18n::trf("ai.error_said", &[("message", &said)]));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, retry_after: Option<&str>) -> HttpResult {
        Ok(HttpResponse {
            status,
            body: String::new(),
            retry_after: retry_after.map(str::to_string),
        })
    }

    /// A busy service is asked again after 2, 4 and 8 seconds, and not a
    /// fifth time.
    #[test]
    fn a_busy_service_is_asked_again_with_growing_waits() {
        for status in [500, 502, 503, 529] {
            let waits: Vec<Option<u64>> = (1..=MAX_TRIES)
                .map(|t| wait_before_retry(t, &answer(status, None)).map(|d| d.as_secs()))
                .collect();
            assert_eq!(waits, vec![Some(2), Some(4), Some(8), None], "{status}");
        }
    }

    /// The service's own wait is kept when it is short; a long one is
    /// left to the user.
    #[test]
    fn the_services_wait_is_followed_when_short() {
        let secs =
            |status, after| wait_before_retry(1, &answer(status, Some(after))).map(|d| d.as_secs());
        assert_eq!(secs(503, "5"), Some(5));
        assert_eq!(secs(503, "30"), Some(30));
        assert_eq!(secs(503, "31"), None);
        assert_eq!(secs(429, "7"), Some(7));
        assert_eq!(secs(429, "3600"), None);
        // A date is not a number of seconds: the usual waits for a busy
        // service, none for a rate limit.
        assert_eq!(secs(503, "Wed, 21 Oct 2026 07:28:00 GMT"), Some(2));
        assert_eq!(secs(429, "Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }

    /// What a wait does not mend is never sent again: a used up quota
    /// (429 with no wait named), a refused key, an unknown model, a
    /// time out, a reply that came, a network failure.
    #[test]
    fn other_answers_are_not_sent_again() {
        for status in [200, 400, 401, 403, 404, 408, 429, 504, 524] {
            assert_eq!(
                wait_before_retry(1, &answer(status, None)),
                None,
                "{status}"
            );
        }
        assert_eq!(wait_before_retry(1, &Err("no route to host".into())), None);
        assert_eq!(wait_before_retry(0, &answer(503, None)), None);
    }

    thread_local! {
        static CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    fn busy_twice(_: &HttpRequest) -> HttpResult {
        let n = CALLS.with(|c| {
            c.set(c.get() + 1);
            c.get()
        });
        if n <= 2 {
            answer(503, None)
        } else {
            answer(200, None)
        }
    }

    fn request() -> HttpRequest {
        HttpRequest {
            method: "POST",
            url: "http://localhost/v1/chat/completions".into(),
            headers: Vec::new(),
            body: "{}".into(),
            bytes: Vec::new(),
        }
    }

    /// A call sends its request again until the service answers, and
    /// says which try it waits for meanwhile.
    #[test]
    fn a_call_tries_again_until_the_service_answers() {
        CALLS.with(|c| c.set(0));
        let mut sent = Vec::new();
        // No wait: the call runs to its end in one poll.
        let mut call = Call::start(request(), 1024, Some(busy_twice));
        match call.poll(Some(busy_twice), Duration::ZERO, &mut sent) {
            Polled::Done(Ok(r), tries) => {
                assert_eq!((r.status, tries), (200, 3));
            }
            _ => panic!("the call should have ended"),
        }
        assert_eq!(sent.len(), 2, "two more requests, the same as the first");
        assert!(sent.iter().all(|r| r == call.request()));
        // With a wait the call stays pending, names the try to come and
        // sends nothing meanwhile.
        CALLS.with(|c| c.set(0));
        let mut sent = Vec::new();
        let mut call = Call::start(request(), 1024, Some(busy_twice));
        assert!(call.retrying().is_none());
        for _ in 0..3 {
            let polled = call.poll(Some(busy_twice), Duration::from_secs(60), &mut sent);
            assert!(matches!(polled, Polled::Pending));
        }
        assert_eq!(call.retrying(), Some((503, 2, MAX_TRIES)));
        assert!(sent.is_empty());
        assert_eq!(CALLS.with(|c| c.get()), 1);
    }

    fn always_busy(_: &HttpRequest) -> HttpResult {
        Ok(HttpResponse {
            status: 503,
            body: r#"[{"error":{"code":503,"message":"The model is overloaded. Please try again later.","status":"UNAVAILABLE"}}]"#.into(),
            retry_after: None,
        })
    }

    /// A service that stays busy gets every try, then the error is
    /// handed on and said plainly with the service's words.
    #[test]
    fn a_service_that_stays_busy_is_given_up_on_with_a_clear_message() {
        let mut sent = Vec::new();
        let mut call = Call::start(request(), 1024, Some(always_busy));
        let Polled::Done(Ok(r), tries) = call.poll(Some(always_busy), Duration::ZERO, &mut sent)
        else {
            panic!("the call should have ended");
        };
        assert_eq!((r.status, tries), (503, MAX_TRIES));
        assert_eq!(sent.len() as u32, MAX_TRIES - 1);
        let message = gave_up(&r, tries).expect("a message");
        assert!(message.contains("503"), "{message}");
        assert!(message.contains(&MAX_TRIES.to_string()), "{message}");
        assert!(message.contains("The model is overloaded"), "{message}");
        // The first answer of a busy service, and other errors, keep
        // their usual messages.
        assert_eq!(gave_up(&r, 1), None);
        let refused = HttpResponse {
            status: 401,
            ..Default::default()
        };
        assert_eq!(gave_up(&refused, 3), None);
    }
}
