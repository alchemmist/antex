use ratatui::backend::Backend;
use ratatui::layout::Position;
use std::io;
use std::io::Write;

pub(super) fn resolve_cursor_position<B>(
    backend: &mut B,
    reported: Option<Position>,
) -> io::Result<Position>
where
    B: Backend<Error = io::Error> + Write,
{
    if let Some(position) = reported {
        return Ok(position);
    }
    let position = Position::new(0, backend.size()?.height.saturating_sub(1));
    backend.set_cursor_position(position)?;
    write!(backend, "\r\n")?;
    Backend::flush(backend)?;
    Ok(position)
}

#[cfg(test)]
#[path = "startup_position_tests.rs"]
mod tests;
