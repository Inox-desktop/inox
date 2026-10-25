//! Headless tests of the login screen (acceptance Q02, D05, Q01, Q03).
//!
//! Everything lives inside the binary because cxx-qt 0.10 links the bridge's
//! generated C++ archive into every target of the package: an integration test in
//! `tests/` is a separate target and fails to link (`undefined reference to
//! 'cxxbridge1$…'`), for the same reason a library target would (KDAB/cxx-qt#770).
//! The binary is therefore the only place that can hold evidence.
//!
//! Three kinds of evidence live here, and a fourth in the modules themselves:
//!
//! - **The projection QML reads** — colours as `#AARRGGBB`, sizes derived from
//!   tokens, strings from Fluent — is asserted next to the code in [`crate::view`].
//! - **The `auth-v1` client**, over a real socket and a hand-written fake boundary,
//!   is [`boundary`] below. It is the same test `tests/boundary.rs` used to be.
//! - **The process-level contract** — the refusal without a boundary, `--help`, an
//!   unknown argument, and `--self-check` not being a way in — is [`cli`].
//! - **The QML module actually loading** is [`qml`]: it needs a process whose
//!   environment sets `QT_QPA_PLATFORM` *before* Qt starts, and mutating this
//!   process's environment would be `unsafe` in edition 2024, which Inox code may not
//!   write. The test binary therefore spawns itself as that child.
//!
//! Run with:
//!
//! ```sh
//! QT_QPA_PLATFORM=offscreen cargo test -p inox-greet
//! ```

/// The `auth-v1` client, over a real socket.
mod boundary {
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;
    use std::time::Duration;

    use inox_auth::wire::{self, Reply, Request};
    use inox_common::secret::Secret;

    use crate::boundary::{self, Report};

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

        /// Starts a server that is merely *listening*: it accepts connections and
        /// holds them open, answering nothing.
        ///
        /// This exists because `Fake::start(Vec::new())` does not mean "a socket
        /// that is there". With an empty script the accept loop runs zero times,
        /// so the `UnixListener` is dropped the instant the thread starts and the
        /// socket path stops accepting connections. `is_available` only checks
        /// that `connect` succeeds, so a test using the empty script was racing
        /// the listener's teardown: green when the connect won, `ECONNREFUSED`
        /// when it lost. A test for "a socket is available" needs a listener that
        /// stays up, not one that is already gone.
        fn start_listening() -> Self {
            let directory = tempfile::tempdir().expect("a temporary directory");
            let socket = directory.path().join("auth.sock");
            let listener = UnixListener::bind(&socket).expect("the socket is created");
            let (_sender, seen) = mpsc::channel();

            thread::spawn(move || {
                // Held so the accepted streams stay open: closing them is fine for
                // `connect`, but holding them makes the fake unmistakably alive for
                // the duration of the test.
                let mut held = Vec::new();
                while let Ok((stream, _)) = listener.accept() {
                    held.push(stream);
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

    #[test]
    fn an_attempt_carries_the_name_the_credential_and_the_identifier() {
        let fake = Fake::start(vec![Answer::Reply(Reply::Accepted { attempt: 7 })]);

        let report = boundary::authenticate(
            &fake.socket,
            7,
            "alice".to_owned(),
            Secret::from_bytes(b"hunter2"),
        );

        assert_eq!(report, Report::Accepted);
        let seen = fake.seen();
        assert_eq!(seen.attempt, 7);
        assert_eq!(seen.username, "alice");
        assert_eq!(seen.credential, b"hunter2");
    }

    #[test]
    fn a_refusal_is_a_refusal_and_not_a_failure() {
        let fake = Fake::start(vec![Answer::Reply(Reply::Rejected { attempt: 1 })]);

        let report = boundary::authenticate(
            &fake.socket,
            1,
            "alice".to_owned(),
            Secret::from_bytes(b"wrong"),
        );

        assert_eq!(report, Report::Rejected);
    }

    #[test]
    fn a_boundary_that_cannot_answer_is_not_a_refusal() {
        let fake = Fake::start(vec![Answer::Reply(Reply::Unavailable {
            attempt: 1,
            reason: inox_auth::wire::UnavailableReason::Failed,
        })]);

        let report = boundary::authenticate(
            &fake.socket,
            1,
            "alice".to_owned(),
            Secret::from_bytes(b"hunter2"),
        );

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

        let report = boundary::start_session(&fake.socket, 12);

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
            boundary::start_session(&fake.socket, 12),
            Report::SessionStarted(false)
        );
    }

    #[test]
    fn a_cancellation_is_sent_to_the_boundary() {
        // The boundary does not answer a cancellation; the connection is simply
        // closed.
        let fake = Fake::start(vec![Answer::Close]);

        boundary::cancel(&fake.socket, 3);

        assert_eq!(fake.seen().attempt, 3);
    }

    #[test]
    fn a_boundary_that_is_not_there_is_unreachable() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let missing = directory.path().join("nonexistent.sock");

        assert_eq!(
            boundary::authenticate(
                &missing,
                1,
                "alice".to_owned(),
                Secret::from_bytes(b"hunter2"),
            ),
            Report::Unreachable
        );
        assert_eq!(boundary::start_session(&missing, 1), Report::Unreachable);
    }

    #[test]
    fn a_boundary_that_closes_without_answering_is_unreachable() {
        let fake = Fake::start(vec![Answer::Close]);

        assert_eq!(
            boundary::authenticate(
                &fake.socket,
                1,
                "alice".to_owned(),
                Secret::from_bytes(b"hunter2"),
            ),
            Report::Unreachable
        );
    }

    #[test]
    fn a_boundary_that_speaks_another_version_is_unreachable() {
        let fake = Fake::start(vec![Answer::OtherVersion(wire::VERSION + 1)]);

        assert_eq!(
            boundary::authenticate(
                &fake.socket,
                1,
                "alice".to_owned(),
                Secret::from_bytes(b"hunter2"),
            ),
            Report::Unreachable,
            "two versions of the contract are two different contracts"
        );
    }

    #[test]
    fn availability_is_about_a_socket_and_not_about_a_promise() {
        // `start_listening`, not `start(Vec::new())`: the latter's listener is
        // dropped before the test connects, which made this assertion a race.
        let fake = Fake::start_listening();
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
}

/// The two catalogs are in step, and the code only asks for messages that exist
/// (acceptance Q03).
mod catalog {
    use std::collections::BTreeSet;

    use inox_common::i18n::{DEFAULT_LOCALE, compare_catalogs, message_ids};

    use crate::i18n::{CATALOGS, EN_US, ZH_CN};

    /// The id prefixes this component owns.
    const PREFIXES: [&str; 1] = ["greet-"];

    /// The id literals that are *not* messages.
    ///
    /// `greet-…` is otherwise a comfortable name for a widget id or a test's
    /// expectation about one, and either in the sources would otherwise read as a
    /// message nobody defines. Empty today, and kept so that the exemption is a
    /// decision someone makes in this file rather than a hole in the pattern.
    const NOT_MESSAGES: [&str; 0] = [];

    #[test]
    fn the_two_catalogs_define_the_same_messages() {
        let diff = compare_catalogs(EN_US, ZH_CN).expect("both catalogs parse");

        assert!(
            diff.is_clean(),
            "en-US and zh-CN disagree: missing {:?}, extra {:?}, placeholders {:?}",
            diff.missing_ids,
            diff.extra_ids,
            diff.placeholder_mismatches,
        );
    }

    #[test]
    fn the_default_locale_ships_a_catalog() {
        assert!(
            CATALOGS
                .iter()
                .any(|(tag, _)| *tag == DEFAULT_LOCALE && !tag.is_empty()),
            "the fallback locale must have a catalog",
        );
    }

    #[test]
    fn every_message_the_code_asks_for_is_defined() {
        let missing: Vec<String> = referenced_ids()
            .difference(&defined_ids())
            .cloned()
            .collect();

        assert!(
            missing.is_empty(),
            "these ids are used in the code but missing from the catalogs: {missing:?}",
        );
    }

    #[test]
    fn every_message_the_catalogs_define_is_used() {
        let unused: Vec<String> = defined_ids()
            .difference(&referenced_ids())
            .cloned()
            .collect();

        assert!(
            unused.is_empty(),
            "these catalog entries are shown by nothing: {unused:?}",
        );
    }

    /// The ids the catalogs define.
    fn defined_ids() -> BTreeSet<String> {
        let mut ids = message_ids(EN_US).expect("the default catalog parses");
        ids.retain(|id| owns(id));

        ids
    }

    /// The ids the crate's own sources mention.
    ///
    /// A source scan rather than a list to maintain: the point is to catch the call
    /// site that says `greet-field-lable`, and only reading the sources does that.
    fn referenced_ids() -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        let sources = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("the crate has a src directory");

        for entry in sources {
            let path = entry.expect("a readable directory entry").path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a readable source file");
            for literal in string_literals(&source) {
                // A bare prefix is not a message: the tests assert that a message id
                // starts with `greet-`, and that literal is a prefix, not an id.
                if !owns(&literal) || PREFIXES.contains(&literal.as_str()) {
                    continue;
                }
                if NOT_MESSAGES.contains(&literal.as_str()) {
                    continue;
                }
                // An id may carry a `.attribute` suffix; the catalog defines the
                // message, and the attribute is selected when it is formatted.
                let id = literal.split('.').next().unwrap_or(&literal).to_owned();
                ids.insert(id);
            }
        }

        ids
    }

    /// Whether an id belongs to this component.
    fn owns(id: &str) -> bool {
        PREFIXES.iter().any(|prefix| id.starts_with(prefix))
    }

    /// The contents of every double-quoted string literal in `source`.
    ///
    /// A deliberately small scanner — enough for this crate's own source, which has
    /// no raw strings containing a message id — rather than a regular expression the
    /// test would then need a dependency for.
    fn string_literals(source: &str) -> Vec<String> {
        let mut literals = Vec::new();
        let mut current: Option<String> = None;
        let mut escaped = false;

        for character in source.chars() {
            match (&mut current, character) {
                (None, '"') => current = Some(String::new()),
                (Some(literal), '"') if !escaped => {
                    literals.push(std::mem::take(literal));
                    current = None;
                }
                (Some(literal), other) => {
                    if escaped {
                        // The escape is kept: `\n` inside a literal is not part of an
                        // id, and no id contains a backslash.
                        literal.push('\\');
                    }
                    literal.push(other);
                }
                (None, _) => {}
            }
            escaped = character == '\\' && !escaped;
        }

        literals
    }

    #[test]
    fn the_source_scanner_finds_what_it_is_looking_for() {
        // Deliberately in another component's prefix: the scanner reads this file
        // too, and a `greet-` example here would be read as an id the code asks for.
        let found = string_literals(r#"let a = "panel-clock"; let b = "other";"#);
        assert_eq!(found, ["panel-clock", "other"]);

        let found = string_literals("let a = 'c'; let b = \"panel-clock\";");
        assert_eq!(found, ["panel-clock"]);
    }

    #[test]
    fn the_component_owns_exactly_one_prefix() {
        assert_eq!(PREFIXES, ["greet-"]);

        for id in defined_ids() {
            assert!(
                !id.contains('.'),
                "`{id}` looks like an attribute path, not a message id"
            );
        }
    }
}

/// The process-level contract, driven through the real argument handling.
mod cli {
    use std::process::{Command, Output};

    /// Environment variable that turns the test binary into a command-line child.
    const CLI_CHILD_MARKER: &str = "INOX_GREET_CLI_CHILD";

    /// Environment variable carrying the child's arguments, separated by `\u{1f}`.
    const CLI_CHILD_ARGS: &str = "INOX_GREET_CLI_ARGS";

    /// The path the refusal test points `INOX_AUTH_SOCKET` at.
    const MISSING_SOCKET: &str = "/nonexistent/inox-auth-test.sock";

    /// Everything the process said, both streams.
    fn said(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr)
    }

    /// Spawn this test binary as a process that behaves like `main`.
    ///
    /// The binary target itself is not built by `cargo test`, and `tests/` cannot
    /// link the cxx-qt bridge, so the child re-runs [`crate::run`] and translates its
    /// result into the exit status `main` would produce. That keeps the
    /// process-level contract — non-zero status plus a readable message — observable
    /// from a unit test.
    fn run_as_process(arguments: &[&str]) -> Output {
        Command::new(std::env::current_exe().expect("the test binary path"))
            .args([
                "--ignored",
                "--exact",
                "--nocapture",
                "headless_tests::cli::cli_child",
            ])
            .env(CLI_CHILD_MARKER, "1")
            .env(CLI_CHILD_ARGS, arguments.join("\u{1f}"))
            .env("INOX_AUTH_SOCKET", MISSING_SOCKET)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .output()
            .expect("the child test process runs")
    }

    /// The child half of [`run_as_process`].
    #[test]
    #[ignore = "spawned as a child process by the command-line tests"]
    fn cli_child() {
        assert_eq!(
            std::env::var(CLI_CHILD_MARKER).as_deref(),
            Ok("1"),
            "this test only runs as the spawned child"
        );

        if let Err(error) = inox_common::logging::init() {
            eprintln!("the child could not start logging: {error}");
        }

        let raw = std::env::var(CLI_CHILD_ARGS).unwrap_or_default();
        let arguments: Vec<String> = if raw.is_empty() {
            Vec::new()
        } else {
            raw.split('\u{1f}').map(str::to_owned).collect()
        };

        match crate::run(arguments.into_iter()) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                // The same shape `main`'s `anyhow::Result` produces.
                eprintln!("Error: {error:#}");
                std::process::exit(1);
            }
        }
    }

    #[test]
    fn the_real_path_refuses_without_an_authentication_boundary() {
        let output = run_as_process(&[]);
        let text = said(&output);

        assert!(!output.status.success(), "{text}");
        assert!(
            text.contains("authentication boundary"),
            "the refusal has to name what is missing: {text}"
        );
        assert!(
            text.contains("inox-auth"),
            "and the process that provides it: {text}"
        );
        assert!(
            !text.contains("Your password"),
            "a refusal is not a login form: {text}"
        );
    }

    #[test]
    fn help_needs_no_display_and_no_boundary() {
        let output = run_as_process(&["--help"]);
        let text = said(&output);

        assert!(output.status.success(), "{text}");
        assert!(text.contains("inox-greet"), "{text}");
        assert!(
            text.contains("--preview"),
            "the mode that opens the interface without a boundary has to be findable: {text}"
        );
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        let output = run_as_process(&["--bogus"]);
        let text = said(&output);

        assert!(!output.status.success(), "{text}");
        assert!(text.contains("unrecognized argument"), "{text}");
    }

    #[test]
    fn a_self_check_is_not_a_third_way_to_log_in() {
        // `--self-check` is a diagnostic of the preview and needs it: refusing the
        // combination is the difference between an evidence path and a switch that
        // opens the login screen without the boundary.
        let refused = crate::run(["--self-check".to_owned()].into_iter());

        assert!(refused.is_err(), "a self-check on its own was accepted");
    }
}

/// The QML module loading on the offscreen platform.
mod qml {
    use std::process::{Command, Output};

    /// Environment variable the parent test sets on the child it spawns.
    const CHILD_MARKER: &str = "INOX_GREET_QML_CHILD";

    /// Line the child prints once the QML tree has loaded and reported ready.
    pub(super) const SELF_CHECK_MARKER: &str = "INOX_GREET_QML_SELF_CHECK_OK";

    /// Spawn this test binary as a process with the offscreen platform.
    ///
    /// `QT_QPA_PLATFORM` has to be in the environment before Qt is initialised, and
    /// setting it in this process would be `unsafe` in edition 2024 (root
    /// AGENTS.md §3.2 forbids that `unsafe`): the child is the honest way to select a
    /// platform from a test.
    fn run_as_process() -> Output {
        Command::new(std::env::current_exe().expect("the test binary path"))
            .args([
                "--ignored",
                "--exact",
                "--nocapture",
                "headless_tests::qml::qml_child",
            ])
            .env(CHILD_MARKER, "1")
            .env("QT_QPA_PLATFORM", "offscreen")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("DISPLAY")
            .output()
            .expect("the child test process runs")
    }

    /// The child half of [`the_login_screen_loads_offscreen`].
    #[test]
    #[ignore = "spawned as a child process by the_login_screen_loads_offscreen"]
    fn qml_child() {
        assert_eq!(
            std::env::var(CHILD_MARKER).as_deref(),
            Ok("1"),
            "this test only runs as the spawned child"
        );

        if let Err(error) = inox_common::logging::init() {
            eprintln!("the child could not start logging: {error}");
        }

        crate::run(["--preview".to_owned(), "--self-check".to_owned()].into_iter())
            .expect("the QML module loads and the tree reports ready");
        assert!(
            crate::cxxqt_object::qml_ready(),
            "the QML tree must confirm it read the Rust projection"
        );

        // The parent cannot see this process's tracing subscriber, so the success
        // marker goes to stdout. `println!` is allowed in tests (root AGENTS.md §3.5).
        println!("{SELF_CHECK_MARKER}");
    }

    #[test]
    fn the_login_screen_loads_offscreen() {
        let output = run_as_process();
        let text = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "the offscreen self-check failed: {text}"
        );
        assert!(
            text.contains(SELF_CHECK_MARKER),
            "the child did not report a successful self-check: {text}"
        );

        // Qt reports QML errors on stderr even when the process survives.
        for marker in [
            "is not a type",
            "Cannot assign",
            "TypeError",
            "Unable to assign",
        ] {
            assert!(!text.contains(marker), "QML reported an error: {text}");
        }
    }
}
