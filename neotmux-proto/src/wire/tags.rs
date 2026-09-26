//! The tag bytes the codec writes before each value to say what kind it is — a draw command, a
//! colour, a key, a cell flag, a message — so the reader knows how to read
//! what follows. One family to a block.

pub(crate) const CMD_TEXT: u8 = 0;
pub(crate) const CMD_RECT: u8 = 1;
pub(crate) const CMD_TINT: u8 = 2;
pub(crate) const CMD_LINE: u8 = 3;

pub(crate) const COLOR_DEFAULT: u8 = 0;
pub(crate) const COLOR_IDX: u8 = 1;
pub(crate) const COLOR_RGB: u8 = 2;

pub(crate) const KEY_CHAR: u8 = 0;
pub(crate) const KEY_ENTER: u8 = 1;
pub(crate) const KEY_TAB: u8 = 2;
pub(crate) const KEY_BACKTAB: u8 = 3;
pub(crate) const KEY_BACKSPACE: u8 = 4;
pub(crate) const KEY_ESC: u8 = 5;
pub(crate) const KEY_UP: u8 = 6;
pub(crate) const KEY_DOWN: u8 = 7;
pub(crate) const KEY_LEFT: u8 = 8;
pub(crate) const KEY_RIGHT: u8 = 9;
pub(crate) const KEY_HOME: u8 = 10;
pub(crate) const KEY_END: u8 = 11;
pub(crate) const KEY_PAGE_UP: u8 = 12;
pub(crate) const KEY_PAGE_DOWN: u8 = 13;
pub(crate) const KEY_INSERT: u8 = 14;
pub(crate) const KEY_DELETE: u8 = 15;
pub(crate) const KEY_F: u8 = 16;

pub(crate) const CELL_BOLD: u8 = 1;
pub(crate) const CELL_ITALIC: u8 = 2;
pub(crate) const CELL_UNDERLINE: u8 = 4;
pub(crate) const CELL_INVERSE: u8 = 8;
pub(crate) const CELL_WIDE: u8 = 16;
pub(crate) const CELL_WIDE_CONTINUATION: u8 = 32;
pub(crate) const CELL_DIM: u8 = 64;

pub(crate) const TO_SERVER_ATTACH: u8 = 0;
pub(crate) const TO_SERVER_KEY: u8 = 1;
pub(crate) const TO_SERVER_RESIZE: u8 = 2;
pub(crate) const TO_SERVER_LIST: u8 = 5;
pub(crate) const TO_SERVER_KILL: u8 = 6;
pub(crate) const TO_SERVER_COMMAND: u8 = 7;

pub(crate) const TO_CLIENT_DONE: u8 = 1;
pub(crate) const TO_CLIENT_REFUSED: u8 = 3;
pub(crate) const TO_CLIENT_CLIP: u8 = 4;
pub(crate) const TO_CLIENT_BYE: u8 = 5;
pub(crate) const TO_CLIENT_SESSIONS: u8 = 6;
pub(crate) const TO_CLIENT_KILLED: u8 = 7;
pub(crate) const TO_CLIENT_RAN: u8 = 8;

pub(crate) const END_QUIT: u8 = 0;
pub(crate) const END_EOF: u8 = 1;
pub(crate) const END_DETACH: u8 = 2;
