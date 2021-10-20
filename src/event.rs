use std::{
    sync::{mpsc, mpsc::Receiver},
    thread,
    time::Duration,
};
use termion::event::Key;
use termion::input::TermRead;

pub enum Event<I> {
    Input(I),
    Tick,
}

pub struct Events {
    rx: mpsc::Receiver<Event<Key>>,
    input_handle: thread::JoinHandle<()>,
    tick_handle: thread::JoinHandle<()>,
}

impl Events {
    pub fn new(tick_dur_recv: Receiver<Duration>) -> Events {
        let (tx, rx) = mpsc::channel();
        let input_handle = {
            let tx = tx.clone();
            thread::spawn(move || match termion::get_tty() {
                Ok(input) => {
                    for key in input.keys().filter_map(|evt| evt.ok()) {
                        if let Err(err) = tx.send(Event::Input(key)) {
                            eprintln!("{}", err);
                            return;
                        }
                    }
                }
                Err(msg) => panic!("{}", msg),
            })
        };
        let tick_handle = {
            thread::spawn(move || loop {
                match tick_dur_recv.recv() {
                    Ok(dur) => {
                        thread::sleep(dur);
                        if let Err(err) = tx.send(Event::Tick) {
                            eprintln!("{}", err);
                            break;
                        }
                    }
                    Err(err) => {
                        eprintln!("{}", err);
                        break;
                    }
                }
            })
        };
        Events {
            rx,
            input_handle,
            tick_handle,
        }
    }

    pub fn next(&self) -> Result<Event<Key>, mpsc::RecvError> {
        self.rx.recv()
    }
}
