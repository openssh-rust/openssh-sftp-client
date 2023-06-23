//! Expected `Cargo.toml` dependencies configuration:
//!
//! ```
//! [dependencies]
//! openssh-sftp-client = { version = "0.13.6", features = ["openssh"] }
//! tokio = { version = "1.11.0", features = ["rt", "macros"] }
//! openssh = { version = "0.9.9", features = ["native-mux"] }
//! ```

use std::error::Error;

use openssh::{KnownHosts, Session as SshSession};
use openssh_sftp_client::Sftp;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let ssh_session = SshSession::connect_mux("ssh://user@hostname:port", KnownHosts::Add).await?;
    let sftp = Sftp::from_session(ssh_session, Default::default()).await?;

    sftp.close().await?;
    Ok(())
}
