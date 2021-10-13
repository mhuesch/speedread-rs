#[allow(dead_code)]
mod event;

use crate::event::{Event, Events};
use std::{cmp, error::Error, io, io::Read, sync::{mpsc, mpsc::{Receiver, Sender}}, thread, time::Duration};
use structopt::StructOpt;
use termion::{event::Key, input::MouseTerminal, raw::IntoRawMode, screen::AlternateScreen};
use tui::{
    backend::TermionBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

#[allow(dead_code)]
const HELP: &str = r#"
########################################################################
#                              __                    __                #
#    _________  ___  ___  ____/ /_______  ____ _____/ /     __________ #
#   / ___/ __ \/ _ \/ _ \/ __  / ___/ _ \/ __ `/ __  /_____/ ___/ ___/ #
#  (__  ) /_/ /  __/  __/ /_/ / /  /  __/ /_/ / /_/ /_____/ /  (__  )  #
# /____/ .___/\___/\___/\__,_/_/   \___/\__,_/\__,_/     /_/  /____/   #
#     /_/                                                              #
#                                                                      #
########################################################################
#                                                                      #
# interactive controls while reading:                                  #
#                                                                      #
#   [ - decrease wpm by 10% (slow down)                                #
#   ] - increase wpm by 10% (speed up)                                 #
#   space - pause                                                      #
#   q - quit                                                           #
#                                                                      #
########################################################################
"#;

struct App {
    text: Vec<String>,
    word_idx: usize,
    /// `None` signifies a paused state. `Some` indicates we are reading,
    /// and the `JoinHandle` refers to our `Tick` thread.
    opt_ticker: Option<(thread::JoinHandle<()>, Sender<Duration>)>,
    wpm: u64,
    tick_send: Sender<Tick>,
    tick_recv: Receiver<Tick>,
}

enum SpeedChange {
    Slower,
    Faster,
}

struct Tick;

fn mk_ticker_handle(tick_dur_recv: Receiver<Duration>, tick_send: Sender<Tick>) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        match tick_dur_recv.recv() {
            Ok(dur) => {
                thread::sleep(dur);
                tick_send.send(Tick).unwrap();
            }
            Err(_err) => {
                break;
            }
        }
    })
}

impl App {
    fn new(init_wpm: u64, text: Vec<String>, resume: usize) -> App {
        let (tick_send, tick_recv) = mpsc::channel();
        let (tick_dur_send, tick_dur_recv) = mpsc::channel();
        let ticker_handle = mk_ticker_handle(tick_dur_recv, tick_send.clone());
        App {
            text,
            word_idx: resume,
            opt_ticker: Some((ticker_handle, tick_dur_send)),
            wpm: init_wpm,
            tick_send,
            tick_recv,
        }
    }

    fn preceding_n_words(&mut self, n: usize) -> Vec<String> {
        let start = cmp::max(self.word_idx - n, 0);
        let slice = &self.text[start..self.word_idx];
        slice.to_vec()
    }

    fn succeeding_n_words(&mut self, n: usize) -> Vec<String> {
        // we must grab an extra word because the slice includes the current
        // word, which we remove.
        let end = cmp::min(self.word_idx + n + 1, self.text.len() - 1);
        let slice = &self.text[self.word_idx..end];
        let mut vec = slice.to_vec();
        if !vec.is_empty() {
            vec.remove(0);
        }
        vec
    }

    fn current_word(&mut self) -> String {
        self.text[self.word_idx].clone()
    }

    fn retreat_a_word(&mut self) {
        self.word_idx = cmp::max(self.word_idx - 1, 0);
    }

    fn advance_a_word(&mut self) {
        self.word_idx = cmp::min(self.word_idx + 1, self.text.len() - 1);
    }

    fn send_current_duration(&self) {
        match &self.opt_ticker {
            None => { }
            Some((_, tick_dur_send)) => {
                let dur = Duration::from_millis(self.standard_tick_millis());
                tick_dur_send.send(dur).unwrap();
            }
        }
    }

    fn standard_tick_millis(&self) -> u64 {
        //  60s  * 1000 ms *  1 min
        // -----   -------   --------
        // 1 min     1 s      x words
        60 * 1000 / self.wpm
    }

    fn toggle(&mut self) {
        let new_opt_ticker = match self.opt_ticker {
            Some(_) => None,
            None => {
                let (tick_dur_send, tick_dur_recv) = mpsc::channel();
                Some((mk_ticker_handle(tick_dur_recv, self.tick_send.clone()), tick_dur_send))
            }
        };
        self.opt_ticker = new_opt_ticker;
        self.send_current_duration();
    }

    fn speed_change(&mut self, v: SpeedChange) {
        self.wpm = match v {
            SpeedChange::Slower => (self.wpm * 9 / 10),
            SpeedChange::Faster => (self.wpm * 11 / 10),
        };
    }

    fn paused(&self) -> bool {
        self.opt_ticker.is_none()
    }
}

#[derive(StructOpt, Debug)]
#[structopt(about = HELP)]
struct Cli {
    /// Desired initial reading speed (words per minute)
    #[structopt(long, short, default_value = "300")]
    wpm: u64,

    /// Desired index of initial word in text
    #[structopt(long, short, default_value = "0")]
    resume: usize,

    /// How many preceding words to show when paused
    #[structopt(long, short, default_value = "3")]
    preceding_word_count: usize,

    /// How many succeeding words to show when paused
    #[structopt(long, short, default_value = "3")]
    succeeding_word_count: usize,
    // #[structopt(long,short)]
    // multiword: bool,
}

fn find_orp(len: usize) -> usize {
    if len < 1 {
        panic!("zero length string");
    } else if len > 13 {
        4
    } else {
        let idxs: [usize; 14] = [0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3];
        idxs[len]
    }
}

fn go(args: Cli) -> Result<(usize, u64), Box<dyn Error>> {
    let mut buffer = String::new();
    std::io::stdin().lock().read_to_string(&mut buffer)?;
    let text = buffer.split_whitespace().map(|x| x.to_string()).collect();

    // Terminal initialization
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let events = Events::new();

    let mut app = App::new(args.wpm, text, args.resume);
    app.send_current_duration();

    loop {
        terminal.draw(|f| {
            let size = f.size();
            let lower = size.height / 2;
            let mid = 3;
            let upper2 = 2;
            let upper = size.height - (upper2 + lower + mid);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(upper),
                        Constraint::Length(upper2),
                        Constraint::Length(mid),
                        Constraint::Length(lower),
                    ]
                    .as_ref(),
                )
                .split(size);
            let create_block = |title| {
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Magenta))
                    .title(Span::styled(
                        title,
                        Style::default().add_modifier(Modifier::BOLD),
                    ))
            };

            //-------------------//
            // Main text widget //
            //-----------------//
            let mut head: Vec<char> = app.current_word().chars().collect();
            let text_len = head.len();
            let orp_idx = find_orp(text_len);
            // this is a bit dodgy, since `split_off` panics if we go out of bounds
            // however _I think_ that `find_orp` ensures the orp point always has at least 1 char
            // to its right.
            let tail = head.split_off(orp_idx + 1);
            let middle = match head.pop() {
                None => Span::raw(""),
                Some(c) => {
                    let mut body = String::new();
                    body.push(c);
                    Span::styled(body, Style::default().fg(Color::Red))
                }
            };
            let padding = " ".repeat(text_len - (2 * orp_idx));
            let head: String = head.into_iter().collect();
            let tail: String = tail.into_iter().collect();
            let line = vec![Spans::from(vec![
                Span::raw(padding),
                Span::raw(head),
                middle,
                Span::raw(tail),
            ])];
            let paragraph = Paragraph::new(line)
                .style(Style::default().fg(Color::Gray))
                .block(create_block("mid"))
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[2]);

            //---------------//
            // upper widget //
            //-------------//
            let line = vec![Spans::from(vec![if app.paused() {
                Span::raw(app.preceding_n_words(args.preceding_word_count).join(" "))
            } else {
                Span::raw("")
            }])];
            let paragraph = Paragraph::new(line)
                .style(Style::default().fg(Color::Gray))
                .block(Block::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);

            //---------------//
            // lower widget //
            //-------------//
            let line = vec![Spans::from(vec![if app.paused() {
                Span::raw(app.succeeding_n_words(args.succeeding_word_count).join(" "))
            } else {
                Span::raw("")
            }])];
            let paragraph = Paragraph::new(line)
                .style(Style::default().fg(Color::Gray))
                .block(Block::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[3]);
        })?;

        match events.next() {
            Ok(Event::Input(input)) => {
                if input == Key::Char('q') {
                    break;
                } else if input == Key::Char(' ') {
                    app.toggle();
                } else if input == Key::Char('[') {
                    app.speed_change(SpeedChange::Slower);
                } else if input == Key::Char(']') {
                    app.speed_change(SpeedChange::Faster);
                } else if input == Key::Left && app.paused() {
                    app.retreat_a_word();
                } else if input == Key::Right && app.paused() {
                    app.advance_a_word();
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                println!("key event channel disconnected");
                break;
            }
        }

        match app.tick_recv.try_recv() {
            Ok(Tick) => {
                if !app.paused() {
                    app.advance_a_word();
                    app.send_current_duration();
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                println!("ticker channel disconnected");
                break;
            }
        }
    }
    Ok((app.word_idx, app.wpm))
}

fn main() {
    let args = Cli::from_args();

    match go(args) {
        Err(msg) => println!("err: {}", msg),
        Ok((final_idx, final_wpm)) => {
            println!(
                "to resume from this point, run with flag `-r {}`. \n\
                 to resume with this speed, run with flag `-w {}`.",
                final_idx, final_wpm
            );
        }
    }
}
