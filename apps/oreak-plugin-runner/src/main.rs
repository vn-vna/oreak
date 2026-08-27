use std::io::{self, BufRead, BufReader, BufWriter, Write};

use oreak_plugin_runner::{MAX_REQUEST_BYTES, Request, Response, RunnerState};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let mut state = RunnerState::default();

    while let Some(line) = read_limited_line(&mut reader, MAX_REQUEST_BYTES)? {
        let (response, shutdown) = match line {
            Ok(line) => match serde_json::from_slice::<Request>(&line) {
                Ok(request) => state.handle(request),
                Err(error) => (Response::invalid_request(error.to_string()), false),
            },
            Err(()) => (
                Response::invalid_request(format!("request exceeds {MAX_REQUEST_BYTES} bytes")),
                false,
            ),
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        if shutdown {
            break;
        }
    }
    Ok(())
}

fn read_limited_line<R: BufRead>(
    reader: &mut R,
    limit: usize,
) -> io::Result<Option<Result<Vec<u8>, ()>>> {
    let mut line = Vec::new();
    let mut too_large = false;
    let mut read_any = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if read_any {
                Ok(Some(if too_large { Err(()) } else { Ok(line) }))
            } else {
                Ok(None)
            };
        }
        read_any = true;
        let chunk_len = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if !too_large {
            if line.len().saturating_add(chunk_len) > limit {
                too_large = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..chunk_len]);
            }
        }
        let ended = available[chunk_len - 1] == b'\n';
        reader.consume(chunk_len);
        if ended {
            if !too_large {
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
            }
            return Ok(Some(if too_large { Err(()) } else { Ok(line) }));
        }
    }
}
