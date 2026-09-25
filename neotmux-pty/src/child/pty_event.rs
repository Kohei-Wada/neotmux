use std::io;

pub enum PtyEvent {
    Output(Vec<u8>),
    Eof,
    Error(io::Error),
}
