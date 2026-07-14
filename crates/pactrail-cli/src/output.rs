use std::io::Write;

/// Writes user output without bypassing I/O error handling.
pub fn write_stdout(value: &str) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(value.as_bytes())?;
    lock.flush()
}

/// Writes diagnostics without using unstructured print macros.
pub fn write_stderr(value: &str) -> std::io::Result<()> {
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    lock.write_all(value.as_bytes())?;
    lock.flush()
}
