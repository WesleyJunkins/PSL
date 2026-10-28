//! Local transport: Unix domain sockets on Unix, TCP loopback elsewhere (docs/01 §Two planes).

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

/// An address as written in `PSL_CONTROL` / `PSL_DATA`: a socket path (starts with `/`) or `host:port`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Address {
    Unix(PathBuf),
    Tcp(String),
}

impl Address {
    pub fn parse(s: &str) -> Address {
        if s.starts_with('/') {
            Address::Unix(PathBuf::from(s))
        } else {
            Address::Tcp(s.to_string())
        }
    }

    pub fn connect(&self) -> io::Result<Conn> {
        match self {
            #[cfg(unix)]
            Address::Unix(p) => Ok(Conn::Unix(UnixStream::connect(p)?)),
            #[cfg(not(unix))]
            Address::Unix(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unix sockets are not available here",
            )),
            Address::Tcp(s) => {
                let addr = s
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad address"))?;
                Ok(Conn::Tcp(TcpStream::connect(addr)?))
            }
        }
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::Unix(p) => write!(f, "{}", p.display()),
            Address::Tcp(s) => write!(f, "{}", s),
        }
    }
}

pub enum Conn {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Conn {
    pub fn try_clone(&self) -> io::Result<Conn> {
        Ok(match self {
            #[cfg(unix)]
            Conn::Unix(s) => Conn::Unix(s.try_clone()?),
            Conn::Tcp(s) => Conn::Tcp(s.try_clone()?),
        })
    }

    pub fn set_read_timeout(&self, d: Option<Duration>) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Conn::Unix(s) => s.set_read_timeout(d),
            Conn::Tcp(s) => s.set_read_timeout(d),
        }
    }

    pub fn shutdown(&self) {
        match self {
            #[cfg(unix)]
            Conn::Unix(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            Conn::Tcp(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Conn::Unix(s) => s.read(buf),
            Conn::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Conn::Unix(s) => s.write(buf),
            Conn::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Conn::Unix(s) => s.flush(),
            Conn::Tcp(s) => s.flush(),
        }
    }
}

pub enum Listener {
    #[cfg(unix)]
    Unix(UnixListener, PathBuf),
    Tcp(TcpListener),
}

impl Listener {
    /// Bind a Unix socket at `path` on Unix; `path` is ignored elsewhere and a loopback TCP port is chosen.
    pub fn bind_local(path: &std::path::Path) -> io::Result<Listener> {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(path);
            Ok(Listener::Unix(
                UnixListener::bind(path)?,
                path.to_path_buf(),
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(Listener::Tcp(TcpListener::bind("127.0.0.1:0")?))
        }
    }

    pub fn address(&self) -> Address {
        match self {
            #[cfg(unix)]
            Listener::Unix(_, p) => Address::Unix(p.clone()),
            Listener::Tcp(l) => Address::Tcp(
                l.local_addr()
                    .map(|a: SocketAddr| a.to_string())
                    .unwrap_or_default(),
            ),
        }
    }

    pub fn accept(&self) -> io::Result<Conn> {
        match self {
            #[cfg(unix)]
            Listener::Unix(l, _) => Ok(Conn::Unix(l.accept()?.0)),
            Listener::Tcp(l) => Ok(Conn::Tcp(l.accept()?.0)),
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Listener::Unix(_, p) = self {
            let _ = std::fs::remove_file(p);
        }
    }
}
