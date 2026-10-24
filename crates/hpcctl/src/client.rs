use anyhow::{Result, bail};
use hpcd_core::ipc::{Request, Response};

#[cfg(unix)]
pub fn call(request: &Request) -> Result<Response> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    use anyhow::Context;

    let path = std::env::var_os("HPCD_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| hpcd_core::ipc::DEFAULT_SOCKET.into());
    let mut stream = UnixStream::connect(&path).with_context(|| {
        format!(
            "connecting to hpcd at {} (is the daemon running?)",
            path.display()
        )
    })?;
    writeln!(stream, "{}", serde_json::to_string(request)?)?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let response: Response = serde_json::from_str(&line).context("reading the answer from hpcd")?;
    if let Response::Error { message } = response {
        bail!(message);
    }
    Ok(response)
}

#[cfg(not(unix))]
pub fn call(request: &Request) -> Result<Response> {
    let _ = request;
    bail!("talking to the hpcd daemon needs Linux")
}
