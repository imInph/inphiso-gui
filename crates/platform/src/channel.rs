//! The local socket between the app and the elevated helper: a Unix socket in
//! a private directory, or a named pipe on Windows. Authenticated by a random
//! token the app passes on the helper's command line.

use std::io;
#[cfg(unix)]
use std::path::PathBuf;

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, Stream};

/// A random hex string, used for socket names and the auth token.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random number generator");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Server side of the channel. Removes its socket directory on drop.
pub struct Server {
    pub listener: Listener,
    /// What the helper passes to [`connect`].
    pub name: String,
    /// Where the helper writes its log, readable by the app afterwards.
    pub log_path: std::path::PathBuf,
    #[cfg(unix)]
    dir: PathBuf,
}

impl Server {
    /// Waits for the helper to connect, polling `give_up` every 50 ms so the
    /// caller can stop waiting (say, the password prompt was cancelled).
    /// Returns `Ok(Err(reason))` if `give_up` did.
    pub fn accept_polling<T>(
        &self,
        mut give_up: impl FnMut() -> Option<T>,
    ) -> io::Result<Result<Stream, T>> {
        use interprocess::local_socket::ListenerNonblockingMode;
        self.listener
            .set_nonblocking(ListenerNonblockingMode::Accept)?;
        loop {
            match self.listener.accept() {
                Ok(stream) => {
                    // On macOS (BSD) an accepted socket inherits O_NONBLOCK from the
                    // listener; reads would then fail with WouldBlock instead of waiting.
                    stream.set_nonblocking(false)?;
                    return Ok(Ok(stream));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            if let Some(reason) = give_up() {
                return Ok(Err(reason));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = std::fs::remove_dir_all(&self.dir);
        #[cfg(windows)]
        let _ = std::fs::remove_file(&self.log_path);
    }
}

#[cfg(unix)]
pub fn listen() -> io::Result<Server> {
    use interprocess::local_socket::GenericFilePath;
    use std::os::unix::fs::DirBuilderExt;

    // A 0700 directory so other users can't reach the socket; root (the helper) still can.
    let dir = std::env::temp_dir().join(format!("inphiso-{}", random_hex(8)));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let path = dir.join("helper.sock");
    let name = path.to_string_lossy().into_owned();
    let listener = ListenerOptions::new()
        .name(path.as_path().to_fs_name::<GenericFilePath>()?)
        .create_sync()?;
    Ok(Server {
        listener,
        name,
        log_path: dir.join("helper.log"),
        dir,
    })
}

#[cfg(windows)]
pub fn listen() -> io::Result<Server> {
    use interprocess::local_socket::GenericNamespaced;

    let name = format!("inphiso-{}", random_hex(16));
    let listener = ListenerOptions::new()
        .name(name.as_str().to_ns_name::<GenericNamespaced>()?)
        .create_sync()?;
    let log_path = std::env::temp_dir().join(format!("{name}.log"));
    Ok(Server {
        listener,
        name,
        log_path,
    })
}

/// Client side, used by the helper.
pub fn connect(name: &str) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        Stream::connect(std::path::Path::new(name).to_fs_name::<GenericFilePath>()?)
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        Stream::connect(name.to_ns_name::<GenericNamespaced>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn round_trip() {
        let server = listen().unwrap();
        let name = server.name.clone();
        let client = std::thread::spawn(move || {
            let mut s = connect(&name).unwrap();
            s.write_all(b"hello\n").unwrap();
        });
        let conn = server.listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(conn).read_line(&mut line).unwrap();
        assert_eq!(line, "hello\n");
        client.join().unwrap();
    }

    /// The accepted stream must block on reads even though the listener polls.
    #[test]
    fn polled_accept_gives_a_blocking_stream() {
        let server = listen().unwrap();
        let name = server.name.clone();
        let client = std::thread::spawn(move || {
            let mut s = connect(&name).unwrap();
            // Connect first, write later: a non-blocking stream would see WouldBlock.
            std::thread::sleep(std::time::Duration::from_millis(300));
            s.write_all(b"late\n").unwrap();
        });
        let conn = server.accept_polling(|| None::<()>).unwrap().unwrap();
        let mut line = String::new();
        BufReader::new(conn).read_line(&mut line).unwrap();
        assert_eq!(line, "late\n");
        client.join().unwrap();
    }

    #[test]
    fn random_hex_is_hex() {
        let h = random_hex(16);
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(h, random_hex(16));
    }
}
