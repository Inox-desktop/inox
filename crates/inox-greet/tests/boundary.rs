//! The login screen's side of `auth-v1`, over a real socket.
//!
//! The server here is hand-written and tiny: it decodes requests with
//! `inox_auth::wire` (the same framing the real boundary uses) and answers with a
//! script. That is enough to check everything this side is responsible for — which
//! request it sends, what it does with each answer, and what it does when the
//! boundary is not there at all — without starting a privileged process or a
//! `pam.d` file. The other half, the real `inox-auth` server, is driven by its own
//! tests.

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use inox_auth::wire::{self, Reply, Request};
use inox_common::secret::Secret;
use inox_greet::boundary::{self, Report};

/// What the fake boundary saw on one connection.
#[derive(Debug)]
struct Seen {
    attempt: u64,
    username: String,
    credential: Vec<u8>,
}

/// What the fake boundary should answer with, per connection.
enum Answer {
    /// A reply to the first request.
    Reply(Reply),
    /// Close the connection without answering.
    Close,
    /// Greet with a version this build does not speak.
    OtherVersion(u32),
}

/// A scripted `auth-v1` server on its own socket.
struct Fake {
    /// Held so the socket's directory lives as long as the server does.
    _directory: tempfile::TempDir,
    socket: PathBuf,
    seen: Receiver<Seen>,
}

impl Fake {
    /// Starts a server that answers each connection with the next item of `script`.
    fn start(script: Vec<Answer>) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("auth.sock");
        let listener = UnixListener::bind(&socket).expect("the socket is created");
        let (sender, seen) = mpsc::channel();

        thread::spawn(move || {
            for answer in script {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };

                if let Err(error) = serve(stream, answer, &sender) {
                    eprintln!("the fake boundary stopped: {error}");
                }
            }
        });

        Self {
            _directory: directory,
            socket,
            seen,
        }
    }

    /// What the boundary saw, in the order the connections arrived.
    fn seen(&self) -> Seen {
        self.seen
            .recv_timeout(Duration::from_secs(5))
            .expect("the boundary saw a request")
    }
}

/// One connection: handshake, one request, one answer.
fn serve(
    mut stream: UnixStream,
    answer: Answer,
    seen: &mpsc::Sender<Seen>,
) -> Result<(), wire::WireError> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

    let Some(Request::Hello { .. }) = wire::read_request(&mut stream)? else {
        return Err(wire::WireError::Malformed("the client did not greet"));
    };

    // The handshake. A version this build does not speak is answered with it and
    // nothing else, which is the one connection that is not a scripted reply.
    let reply_to_the_request = match answer {
        Answer::OtherVersion(version) => {
            wire::write_reply(&mut stream, &Reply::Welcome { version })?;
            return Ok(());
        }
        other => other,
    };

    wire::write_reply(
        &mut stream,
        &Reply::Welcome {
            version: wire::VERSION,
        },
    )?;

    match wire::read_request(&mut stream)? {
        Some(Request::Authenticate {
            attempt,
            username,
            credential,
        }) => {
            let _ = seen.send(Seen {
                attempt,
                username,
                credential: credential.expose().to_vec(),
            });
        }
        Some(Request::StartSession { attempt }) | Some(Request::Cancel { attempt }) => {
            let _ = seen.send(Seen {
                attempt,
                username: String::new(),
                credential: Vec::new(),
            });
        }
        Some(Request::Hello { .. }) | None => {
            return Err(wire::WireError::Malformed("the client sent no request"));
        }
    }

    if let Answer::Reply(reply) = reply_to_the_request {
        wire::write_reply(&mut stream, &reply)?;
    }

    Ok(())
}

/// Runs one of the client's futures to completion.
///
/// The transport is blocking I/O inside an `async fn` (see `inox_greet::boundary`),
/// so a plain blocking executor is the honest way to drive it here.
fn run<F: std::future::Future>(future: F) -> F::Output {
    iced::futures::executor::block_on(future)
}

#[test]
fn an_attempt_carries_the_name_the_credential_and_the_identifier() {
    let fake = Fake::start(vec![Answer::Reply(Reply::Accepted { attempt: 7 })]);

    let report = run(boundary::authenticate(
        fake.socket.clone(),
        7,
        "alice".to_owned(),
        Secret::from_bytes(b"hunter2"),
    ));

    assert_eq!(report, Report::Accepted);
    let seen = fake.seen();
    assert_eq!(seen.attempt, 7);
    assert_eq!(seen.username, "alice");
    assert_eq!(seen.credential, b"hunter2");
}

#[test]
fn a_refusal_is_a_refusal_and_not_a_failure() {
    let fake = Fake::start(vec![Answer::Reply(Reply::Rejected { attempt: 1 })]);

    let report = run(boundary::authenticate(
        fake.socket.clone(),
        1,
        "alice".to_owned(),
        Secret::from_bytes(b"wrong"),
    ));

    assert_eq!(report, Report::Rejected);
}

#[test]
fn a_boundary_that_cannot_answer_is_not_a_refusal() {
    let fake = Fake::start(vec![Answer::Reply(Reply::Unavailable {
        attempt: 1,
        reason: inox_auth::wire::UnavailableReason::Failed,
    })]);

    let report = run(boundary::authenticate(
        fake.socket.clone(),
        1,
        "alice".to_owned(),
        Secret::from_bytes(b"hunter2"),
    ));

    assert_eq!(
        report,
        Report::Unavailable,
        "the screen has to be able to say 'this build cannot check a login', not \
         'your password is wrong'"
    );
}

#[test]
fn a_session_start_is_its_own_connection_and_its_own_request() {
    let fake = Fake::start(vec![Answer::Reply(Reply::SessionStarted {
        attempt: 12,
        started: true,
    })]);

    let report = run(boundary::start_session(fake.socket.clone(), 12));

    assert_eq!(report, Report::SessionStarted(true));
    assert_eq!(fake.seen().attempt, 12);
}

#[test]
fn a_session_that_did_not_start_is_reported_as_such() {
    let fake = Fake::start(vec![Answer::Reply(Reply::SessionStarted {
        attempt: 12,
        started: false,
    })]);

    assert_eq!(
        run(boundary::start_session(fake.socket.clone(), 12)),
        Report::SessionStarted(false)
    );
}

#[test]
fn a_cancellation_is_sent_to_the_boundary() {
    // The boundary does not answer a cancellation; the connection is simply closed.
    let fake = Fake::start(vec![Answer::Close]);

    run(boundary::cancel(fake.socket.clone(), 3));

    assert_eq!(fake.seen().attempt, 3);
}

#[test]
fn a_boundary_that_is_not_there_is_unreachable() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let missing = directory.path().join("nonexistent.sock");

    assert_eq!(
        run(boundary::authenticate(
            missing.clone(),
            1,
            "alice".to_owned(),
            Secret::from_bytes(b"hunter2"),
        )),
        Report::Unreachable
    );
    assert_eq!(
        run(boundary::start_session(missing, 1)),
        Report::Unreachable
    );
}

#[test]
fn a_boundary_that_closes_without_answering_is_unreachable() {
    let fake = Fake::start(vec![Answer::Close]);

    assert_eq!(
        run(boundary::authenticate(
            fake.socket.clone(),
            1,
            "alice".to_owned(),
            Secret::from_bytes(b"hunter2"),
        )),
        Report::Unreachable
    );
}

#[test]
fn a_boundary_that_speaks_another_version_is_unreachable() {
    let fake = Fake::start(vec![Answer::OtherVersion(wire::VERSION + 1)]);

    assert_eq!(
        run(boundary::authenticate(
            fake.socket.clone(),
            1,
            "alice".to_owned(),
            Secret::from_bytes(b"hunter2"),
        )),
        Report::Unreachable,
        "two versions of the contract are two different contracts"
    );
}

#[test]
fn availability_is_about_a_socket_and_not_about_a_promise() {
    let fake = Fake::start(Vec::new());
    assert!(boundary::is_available(&fake.socket));

    let directory = tempfile::tempdir().expect("a temporary directory");
    assert!(!boundary::is_available(
        &directory.path().join("nothing.sock")
    ));
}

#[test]
fn the_request_the_screen_sends_is_one_the_boundary_can_decode() {
    // The other direction of the same property the boundary's own tests check: a
    // request this crate builds has to be one the framing accepts, including the
    // account-name grammar and the credential bound.
    let request = Request::Authenticate {
        attempt: 1,
        username: "alice-1.test".to_owned(),
        credential: Secret::from_bytes(b"hunter2"),
    };

    let encoded = request.encode().expect("the request encodes");
    let decoded = wire::decode_request(&encoded).expect("the request decodes");

    let Request::Authenticate {
        username,
        credential,
        ..
    } = decoded
    else {
        panic!("an authenticate request");
    };
    assert_eq!(username, "alice-1.test");
    assert_eq!(credential.expose(), b"hunter2");
}
