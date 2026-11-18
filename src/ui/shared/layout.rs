use ratatui::Frame;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render_small_terminal_message(frame: &mut Frame<'_>) {
    let paragraph = Paragraph::new("nuvem needs at least 60x12")
        .block(Block::default().title("nuvem").borders(Borders::ALL));
    frame.render_widget(paragraph, frame.area());
}
