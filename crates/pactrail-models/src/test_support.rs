use std::io::{Error, ErrorKind, Read};
use std::net::TcpStream;

pub(crate) fn read_http_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    const LIMIT: usize = 64 * 1024;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if request.len().saturating_add(read) > LIMIT {
            return Err(Error::new(ErrorKind::InvalidData, "request too large"));
        }
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let header_bytes = header_end.saturating_add(4);
        let headers = std::str::from_utf8(&request[..header_end])
            .map_err(|_| Error::new(ErrorKind::InvalidData, "invalid request headers"))?;
        let content_length = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        });
        if content_length.is_none_or(|length| request.len() >= header_bytes + length) {
            return Ok(request);
        }
    }
    Ok(request)
}
