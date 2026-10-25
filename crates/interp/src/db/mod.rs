//! Database client layer shared by PDO and mysqli: wire-protocol drivers
//! (on the runtime's sockets, so a query parks only its green thread) and
//! the result model they produce.

// Parts of the client API serve mysqli (ping, select_db, metadata, ...).
#[allow(dead_code)]
pub mod mysql;
pub mod sql;

/// A driver-level error, as PDO reports it.
#[derive(Debug, Clone)]
pub struct DbError {
    pub sqlstate: String,
    pub code: i64,
    pub message: String,
}

impl DbError {
    pub fn new(sqlstate: &str, code: i64, message: impl Into<String>) -> DbError {
        DbError { sqlstate: sqlstate.to_string(), code, message: message.into() }
    }
}

/// A raw column value from the server, before PHP conversion.
#[derive(Debug, Clone)]
pub enum Cell {
    Null,
    Int(i64),
    UInt(u64),
    Float(f64),
    /// FLOAT columns (4 bytes): PHP rounds them to FLT_DIG digits.
    Float32(f32),
    Bytes(Vec<u8>),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct Column {
    pub name: String,
    pub table: String,
    pub org_table: String,
    pub schema: String,
    pub org_name: String,
    /// Protocol type code (MYSQL_TYPE_*).
    pub kind: u8,
    pub flags: u16,
    pub length: u32,
    pub decimals: u8,
    pub charset: u16,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    pub affected: u64,
    pub insert_id: u64,
    pub warnings: u16,
    pub info: String,
}
