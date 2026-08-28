//! Device-flow login, session storage, and connecting to a console.

use cliclack::{confirm, intro, log, outro, spinner};
use keyring::Entry;
use terminal_link::Link;
use tracel::console::{Console, DeviceLogin, Env, SessionToken, TracelCredentials};

use crate::display;

/// Identifies this example to the device authorization flow and scopes its keyring entries.
const CLIENT_ID: &str = "tracel-console-example";

/// Signs in through the OAuth device flow and saves the approved session.
pub fn login(env: Env) -> anyhow::Result<()> {
    let login = DeviceLogin::start(env.clone(), CLIENT_ID)?;

    intro("Tracel device login")?;
    let link = Link::new(
        &login.verification_uri_complete,
        &login.verification_uri_complete,
    );
    log::step(format!("Open {link}"))?;
    log::info(format!("Enter code {}", login.user_code))?;

    if confirm("Open in your browser?")
        .initial_value(true)
        .interact()?
    {
        webbrowser::open(&login.verification_uri_complete)?;
    }

    let progress = spinner();
    progress.start("Waiting for approval — Ctrl+C to cancel");
    let token = match login.wait_for_approval() {
        Ok(token) => {
            progress.stop("Approved!");
            token
        }
        Err(error) => {
            progress.error("Login failed");
            return Err(error.into());
        }
    };

    let credentials = TracelCredentials::session_token(token.clone());
    let console = Console::connect(env.clone(), &credentials)?;

    session_entry(&env)?.set_password(token.as_str())?;
    display::current_user(&console)?;
    outro("Session saved in the system credential store")?;
    Ok(())
}

/// Clears the session saved for `env`, if any.
pub fn logout(env: Env) -> anyhow::Result<()> {
    match session_entry(&env)?.delete_credential() {
        Ok(()) => log::success("Signed out")?,
        Err(keyring::Error::NoEntry) => log::info("Already signed out")?,
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Connects using `TRACEL_API_KEY`/`TRACEL_SESSION_TOKEN` when set, falling back to the session
/// saved by [`login`].
pub fn connect(env: Env) -> anyhow::Result<Console> {
    let credentials = match TracelCredentials::from_env() {
        Ok(credentials) => credentials,
        Err(_) => {
            let token = session_entry(&env)?.get_password().map_err(|error| {
                anyhow::anyhow!("run `console login` first (credential store: {error})")
            })?;
            TracelCredentials::session_token(SessionToken::new(token))
        }
    };
    Console::connect(env, &credentials).map_err(Into::into)
}

fn session_entry(env: &Env) -> anyhow::Result<Entry> {
    Entry::new(CLIENT_ID, env.get_url().as_str()).map_err(Into::into)
}
