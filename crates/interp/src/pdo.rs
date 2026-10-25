//! PDO and PDOStatement (natives; the class shells, constants and the
//! by-reference binders are in prelude_pdo.php). Drivers: mysql.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::class::Class;
use crate::db::mysql::{self, Conn, Param};
use crate::db::sql::{self, Piece};
use crate::db::{Cell, Column, DbError, ResultSet};
use crate::interp::{BuiltinFn, Callable, Frame, Interp, NativeMethod, R};
use crate::value::{pstr, Internal, Key, NativeData, Obj, PhpArray, Value};

// PDO constants used here.
const PARAM_NULL: i64 = 0;
const PARAM_INT: i64 = 1;
const PARAM_STR: i64 = 2;
const PARAM_LOB: i64 = 3;
const PARAM_BOOL: i64 = 5;
const FETCH_DEFAULT: i64 = 0;
const FETCH_LAZY: i64 = 1;
const FETCH_ASSOC: i64 = 2;
const FETCH_NUM: i64 = 3;
const FETCH_BOTH: i64 = 4;
const FETCH_OBJ: i64 = 5;
const FETCH_BOUND: i64 = 6;
const FETCH_COLUMN: i64 = 7;
const FETCH_CLASS: i64 = 8;
const FETCH_INTO: i64 = 9;
const FETCH_FUNC: i64 = 10;
const FETCH_NAMED: i64 = 11;
const FETCH_KEY_PAIR: i64 = 12;
const FETCH_GROUP: i64 = 32;
const FETCH_UNIQUE: i64 = 64;
const FETCH_CLASSTYPE: i64 = 128;
const FETCH_PROPS_LATE: i64 = 256;
/// Flag bits above the base mode (GROUP, UNIQUE, CLASSTYPE, PROPS_LATE, SERIALIZE).
const FETCH_FLAGS: i64 = !0xF;
const ATTR_AUTOCOMMIT: i64 = 0;
const ATTR_TIMEOUT: i64 = 2;
const ATTR_ERRMODE: i64 = 3;
const ATTR_SERVER_VERSION: i64 = 4;
const ATTR_CLIENT_VERSION: i64 = 5;
const ATTR_SERVER_INFO: i64 = 6;
const ATTR_CONNECTION_STATUS: i64 = 7;
const ATTR_CASE: i64 = 8;
const ATTR_ORACLE_NULLS: i64 = 11;
const ATTR_PERSISTENT: i64 = 12;
const ATTR_STATEMENT_CLASS: i64 = 13;
const ATTR_DRIVER_NAME: i64 = 16;
const ATTR_STRINGIFY_FETCHES: i64 = 17;
const ATTR_EMULATE_PREPARES: i64 = 20;
const ATTR_DEFAULT_FETCH_MODE: i64 = 19;
const ATTR_DEFAULT_STR_PARAM: i64 = 21;
const ERRMODE_WARNING: i64 = 1;
const ERRMODE_EXCEPTION: i64 = 2;
const CASE_NATURAL: i64 = 0;
const CASE_LOWER: i64 = 2;
const CASE_UPPER: i64 = 1;
const NULL_EMPTY_STRING: i64 = 1;
const NULL_TO_STRING: i64 = 2;
const MYSQL_ATTR_USE_BUFFERED_QUERY: i64 = 1000;
const MYSQL_ATTR_LOCAL_INFILE: i64 = 1001;
const MYSQL_ATTR_INIT_COMMAND: i64 = 1002;
const MYSQL_ATTR_FOUND_ROWS: i64 = 1005;
const MYSQL_ATTR_SSL_KEY: i64 = 1007;
const MYSQL_ATTR_SSL_CERT: i64 = 1008;
const MYSQL_ATTR_SSL_CA: i64 = 1009;
const MYSQL_ATTR_SSL_CAPATH: i64 = 1010;
const MYSQL_ATTR_MULTI_STATEMENTS: i64 = 1013;
const MYSQL_ATTR_SSL_VERIFY_SERVER_CERT: i64 = 1014;

#[derive(Clone)]
struct Attrs {
    errmode: i64,
    case: i64,
    oracle_nulls: i64,
    stringify: bool,
    emulate: bool,
    default_fetch: i64,
    statement_class: Option<(Rc<Class>, Vec<Value>)>,
    autocommit: bool,
    persistent: bool,
    timeout: i64,
    default_str: i64,
    buffered: bool,
}

/// A connection: shared by the PDO object and its statements.
pub struct Db {
    conn: Conn,
    attrs: Attrs,
    driver: &'static str,
    /// errorCode()/errorInfo() of the PDO object.
    error: Option<DbError>,
    last_ok: bool,
    in_tx: bool,
}

type Shared = Rc<RefCell<Db>>;

struct PdoState(Shared);

impl NativeData for PdoState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum PKey {
    Pos(usize),
    Name(String),
}

#[derive(Clone)]
struct Bound {
    value: Value,
    kind: i64,
}

#[derive(Clone)]
struct FetchMode {
    mode: i64,
    /// FETCH_COLUMN: column; FETCH_CLASS: class and constructor args;
    /// FETCH_INTO: the object.
    column: usize,
    class: Option<Rc<Class>>,
    ctor_args: Vec<Value>,
    into: Option<Rc<Obj>>,
}

impl FetchMode {
    fn plain(mode: i64) -> FetchMode {
        FetchMode { mode, column: 0, class: None, ctor_args: vec![], into: None }
    }
}

struct StmtState {
    db: Shared,
    /// Keeps the PDO object alive while the statement lives.
    _pdo: Rc<Obj>,
    sql: Vec<u8>,
    pieces: Vec<Piece>,
    /// Named placeholders in order (native prepares send `?`).
    names: Vec<String>,
    positional: usize,
    server: Option<mysql::Stmt>,
    bound: HashMap<PKey, Bound>,
    result: Option<ResultSet>,
    cursor: usize,
    row_count: i64,
    fetch: FetchMode,
    error: Option<DbError>,
    executed: bool,
}

struct StmtData(Rc<RefCell<StmtState>>);

impl NativeData for StmtData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

impl Drop for StmtState {
    fn drop(&mut self) {
        if let Some(st) = self.server.take() {
            if let Ok(mut db) = self.db.try_borrow_mut() {
                db.conn.close_stmt(st.id);
            }
        }
    }
}

fn sqlstate_text(state: &str) -> &'static str {
    match state {
        "00000" => "No error",
        "01000" => "Warning",
        "01004" => "String data, right truncated",
        "07001" => "Wrong number of parameters",
        "08001" => "Client unable to establish connection",
        "08003" => "Connection does not exist",
        "08004" => "Server rejected the connection",
        "08S01" => "Communication link failure",
        "21S01" => "Insert value list does not match column list",
        "22001" => "String data, right truncated",
        "22003" => "Numeric value out of range",
        "22007" => "Invalid datetime format",
        "22008" => "Datetime field overflow",
        "22012" => "Division by zero",
        "22018" => "Invalid character value for cast specification",
        "23000" => "Integrity constraint violation",
        "24000" => "Invalid cursor state",
        "25000" => "Invalid transaction state",
        "28000" => "Invalid authorization specification",
        "3D000" => "Invalid catalog name",
        "40001" => "Serialization failure",
        "42000" => "Syntax error or access violation",
        "42S01" => "Base table or view already exists",
        "42S02" => "Base table or view not found",
        "42S21" => "Column already exists",
        "42S22" => "Column not found",
        "HY000" => "General error",
        "HY001" => "Memory allocation error",
        "HY008" => "Operation canceled",
        "HY010" => "Function sequence error",
        "HY093" => "Invalid parameter number",
        "HYT00" => "Timeout expired",
        "IM001" => "Driver does not support this function",
        "IMSSP" => "Driver specific error",
        _ => "<<Unknown error>>",
    }
}

fn error_message(e: &DbError) -> String {
    if e.code == 0 {
        // PDO's own errors: no driver code; the detail is optional.
        return if e.message.is_empty() {
            format!("SQLSTATE[{}]: {}", e.sqlstate, sqlstate_text(&e.sqlstate))
        } else {
            format!("SQLSTATE[{}]: {}: {}", e.sqlstate, sqlstate_text(&e.sqlstate), e.message)
        };
    }
    format!("SQLSTATE[{}]: {}: {} {}", e.sqlstate, sqlstate_text(&e.sqlstate), e.code, e.message)
}

/// A PDOException for `e` (code = SQLSTATE, errorInfo set).
fn exception(it: &Interp, e: &DbError, msg: Option<String>) -> crate::interp::Ctrl {
    let ctrl = it.error("PDOException", &msg.unwrap_or_else(|| error_message(e)));
    if let crate::interp::Ctrl::Throw(o) = &ctrl {
        o.set_prop("code", Value::str(&e.sqlstate));
        let info = if e.code == 0 && e.message.is_empty() {
            PhpArray::from_list([Value::str(&e.sqlstate)])
        } else {
            PhpArray::from_list([Value::str(&e.sqlstate), if e.code == 0 { Value::Null } else { Value::Int(e.code) }, Value::str(&e.message)])
        };
        o.set_prop("errorInfo", Value::array(info));
    }
    ctrl
}

/// Report a failure per ERRMODE: throw, warn, or stay silent (false).
fn fail(it: &Interp, errmode: i64, func: &str, e: &DbError) -> R<Value> {
    match errmode {
        ERRMODE_EXCEPTION => Err(exception(it, e, None)),
        ERRMODE_WARNING => {
            it.warn(&format!("{func}(): {}", error_message(e)));
            Ok(Value::Bool(false))
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn pdo_of(it: &Interp, this: Option<&Rc<Obj>>) -> R<Shared> {
    let o = this.ok_or_else(|| it.error("Error", "PDO method called statically"))?;
    match &*o.internal() {
        Internal::Data(d) => d.as_any().downcast_ref::<PdoState>().map(|s| s.0.clone()),
        _ => None,
    }
    .ok_or_else(|| it.error("Error", "PDO object is uninitialized"))
}

fn stmt_of(it: &Interp, this: Option<&Rc<Obj>>) -> R<Rc<RefCell<StmtState>>> {
    let o = this.ok_or_else(|| it.error("Error", "PDOStatement method called statically"))?;
    match &*o.internal() {
        Internal::Data(d) => d.as_any().downcast_ref::<StmtData>().map(|s| s.0.clone()),
        _ => None,
    }
    .ok_or_else(|| it.error("Error", "PDOStatement object is uninitialized"))
}

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or_default()
}

fn int_arg(it: &Interp, a: &[Value], i: usize, d: i64) -> R<i64> {
    match a.get(i) {
        None | Some(Value::Null) => Ok(d),
        Some(v) => it.to_int(v),
    }
}

fn str_arg(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&it.to_str(&arg(a, i))?).into_owned())
}

// ---- DSN and connecting ----------------------------------------------------------------

fn parse_dsn(dsn: &str) -> (String, HashMap<String, String>) {
    let (driver, rest) = dsn.split_once(':').unwrap_or((dsn, ""));
    let mut kv = HashMap::new();
    for part in rest.split(';') {
        if let Some((k, v)) = part.split_once('=') {
            kv.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    (driver.to_ascii_lowercase(), kv)
}

fn default_attrs(driver: &str) -> Attrs {
    Attrs {
        errmode: ERRMODE_EXCEPTION,
        case: CASE_NATURAL,
        oracle_nulls: 0,
        stringify: false,
        emulate: driver == "mysql",
        default_fetch: FETCH_BOTH,
        statement_class: None,
        autocommit: true,
        persistent: false,
        timeout: 30,
        default_str: PARAM_STR,
        buffered: true,
    }
}

fn pdo_construct(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.ok_or_else(|| it.error("Error", "PDO::__construct called statically"))?.clone();
    let dsn = str_arg(it, &a, 0)?;
    let user = if arg(&a, 1).is_null() { String::new() } else { str_arg(it, &a, 1)? };
    let pass = if arg(&a, 2).is_null() { String::new() } else { str_arg(it, &a, 2)? };
    let options: Vec<(i64, Value)> = match arg(&a, 3) {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|(k, v)| match k {
                Key::Int(n) => Some((n, v.clone())),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
    let (driver, kv) = parse_dsn(&dsn);
    if !dsn.contains(':') {
        return Err(it.error("PDOException", "PDO::__construct(): Argument #1 ($dsn) must be a valid data source name"));
    }
    match driver.as_str() {
        "mysql" => {}
        _ => return Err(it.error("PDOException", "could not find driver")),
    }
    let mut attrs = default_attrs("mysql");
    let mut opts = mysql::Opts {
        host: kv.get("host").cloned().unwrap_or_else(|| "localhost".into()),
        port: kv.get("port").and_then(|p| p.parse().ok()).unwrap_or(3306),
        socket: kv.get("unix_socket").cloned(),
        user,
        pass,
        db: kv.get("dbname").cloned(),
        charset: kv.get("charset").cloned(),
        timeout: None,
        found_rows: false,
        multi_statements: true,
        local_infile: false,
        ssl: None,
        init_command: None,
    };
    if opts.host == "localhost" && opts.socket.is_none() {
        // mysqlnd's "localhost" means the default Unix socket.
        let sock = it.ini_get("pdo_mysql.default_socket").filter(|s| !s.is_empty()).unwrap_or_else(|| "/var/run/mysqld/mysqld.sock".into());
        opts.socket = Some(sock);
    }
    let mut ssl: Option<phpx_runtime::tls::Options> = None;
    for (k, v) in &options {
        let to_bool = |v: &Value| v.truthy();
        match *k {
            ATTR_ERRMODE => attrs.errmode = it.to_int(v)?,
            ATTR_CASE => attrs.case = it.to_int(v)?,
            ATTR_ORACLE_NULLS => attrs.oracle_nulls = it.to_int(v)?,
            ATTR_STRINGIFY_FETCHES => attrs.stringify = to_bool(v),
            ATTR_EMULATE_PREPARES => attrs.emulate = to_bool(v),
            ATTR_DEFAULT_FETCH_MODE => attrs.default_fetch = it.to_int(v)?,
            ATTR_PERSISTENT => attrs.persistent = to_bool(v),
            ATTR_TIMEOUT => {
                attrs.timeout = it.to_int(v)?;
                opts.timeout = Some(std::time::Duration::from_secs(attrs.timeout.max(0) as u64));
            }
            ATTR_AUTOCOMMIT => attrs.autocommit = to_bool(v),
            MYSQL_ATTR_INIT_COMMAND => opts.init_command = Some(String::from_utf8_lossy(&it.to_str(v)?).into_owned()),
            MYSQL_ATTR_FOUND_ROWS => opts.found_rows = to_bool(v),
            MYSQL_ATTR_MULTI_STATEMENTS => opts.multi_statements = to_bool(v),
            MYSQL_ATTR_LOCAL_INFILE => opts.local_infile = to_bool(v),
            MYSQL_ATTR_USE_BUFFERED_QUERY => attrs.buffered = to_bool(v),
            MYSQL_ATTR_SSL_CA | MYSQL_ATTR_SSL_CAPATH | MYSQL_ATTR_SSL_CERT | MYSQL_ATTR_SSL_KEY => {
                let s = ssl.get_or_insert_with(Default::default);
                let p = String::from_utf8_lossy(&it.to_str(v)?).into_owned();
                if *k == MYSQL_ATTR_SSL_CA {
                    s.cafile = Some(p.into());
                } else if *k == MYSQL_ATTR_SSL_CAPATH {
                    s.capath = Some(p.into());
                }
            }
            MYSQL_ATTR_SSL_VERIFY_SERVER_CERT => {
                let s = ssl.get_or_insert_with(Default::default);
                s.verify_peer = to_bool(v);
                s.verify_peer_name = to_bool(v);
            }
            _ => {}
        }
    }
    if let Some(s) = &mut ssl {
        // mysqlnd verifies only when asked to.
        if !options.iter().any(|(k, _)| *k == MYSQL_ATTR_SSL_VERIFY_SERVER_CERT) {
            s.verify_peer = false;
            s.verify_peer_name = false;
        }
    }
    opts.ssl = ssl;
    let conn = match Conn::connect(&opts) {
        Ok(c) => c,
        Err(e) => {
            let ctrl = it.error("PDOException", &format!("SQLSTATE[HY000] [{}] {}", e.code, e.message));
            if let crate::interp::Ctrl::Throw(x) = &ctrl {
                x.set_prop("code", Value::Int(e.code));
            }
            return Err(ctrl);
        }
    };
    let db = Db { conn, attrs, driver: "mysql", error: None, last_ok: false, in_tx: false };
    o.set_internal(Internal::Data(Box::new(PdoState(Rc::new(RefCell::new(db))))));
    Ok(Value::Null)
}

// ---- values ---------------------------------------------------------------------------------

/// PHP's FLOAT handling: round to FLT_DIG significant digits.
fn float32(f: f32, decimals: u8) -> f64 {
    let s = if decimals < 31 { format!("{:.*}", decimals as usize, f) } else { format!("{:.5e}", f) };
    s.parse().unwrap_or(f as f64)
}

/// A server value as PHP sees it (mysqlnd with native int/float types).
fn to_php(it: &Interp, c: &Cell, col: &Column, attrs: &Attrs) -> Value {
    let v = match c {
        Cell::Null => {
            return if attrs.oracle_nulls == NULL_TO_STRING { Value::str("") } else { Value::Null };
        }
        Cell::Int(n) => Value::Int(*n),
        Cell::UInt(u) => Value::str(&u.to_string()),
        Cell::Float(f) => Value::Float(*f),
        Cell::Float32(f) => Value::Float(float32(*f, col.decimals)),
        Cell::Bytes(b) => {
            if b.is_empty() && attrs.oracle_nulls == NULL_EMPTY_STRING {
                return Value::Null;
            }
            if !attrs.stringify {
                match col.kind {
                    mysql::TYPE_TINY | mysql::TYPE_SHORT | mysql::TYPE_LONG | mysql::TYPE_INT24 | mysql::TYPE_LONGLONG | mysql::TYPE_YEAR => {
                        if let Some(n) = std::str::from_utf8(b).ok().and_then(|s| s.parse::<i64>().ok()) {
                            return Value::Int(n);
                        }
                    }
                    mysql::TYPE_FLOAT | mysql::TYPE_DOUBLE => {
                        if let Some(f) = std::str::from_utf8(b).ok().and_then(|s| s.parse::<f64>().ok()) {
                            return Value::Float(f);
                        }
                    }
                    _ => {}
                }
            }
            return Value::bytes(b.clone());
        }
    };
    if attrs.stringify {
        match v {
            Value::Int(n) => Value::str(&n.to_string()),
            Value::Float(f) => Value::str(&crate::value::fmt_float(f)),
            v => v,
        }
    } else {
        let _ = it;
        v
    }
}

fn column_name(col: &Column, attrs: &Attrs) -> String {
    match attrs.case {
        CASE_LOWER => col.name.to_lowercase(),
        CASE_UPPER => col.name.to_uppercase(),
        _ => col.name.clone(),
    }
}

/// A PHP value bound with PDO type `kind`, for the server.
fn to_param(it: &Interp, v: &Value, kind: i64) -> R<Param> {
    let v = v.deref();
    if v.is_null() {
        return Ok(Param::Null);
    }
    Ok(match kind & 0x7FFF_FFFF {
        PARAM_NULL => Param::Null,
        PARAM_INT => match v {
            Value::Int(n) => Param::Int(n),
            Value::Bool(b) => Param::Int(b as i64),
            Value::Float(f) => Param::Int(f as i64),
            other => Param::Bytes(it.to_str(&other)?.to_vec()),
        },
        PARAM_BOOL => Param::Int(v.truthy() as i64),
        PARAM_LOB => match v {
            Value::Resource(_) => {
                let data = (it.builtins["stream_get_contents"])(it, &mut Frame::new(it.file_ctx(0), 0), &mut [v.clone()])?;
                Param::Bytes(it.to_str(&data)?.to_vec())
            }
            other => Param::Bytes(it.to_str(&other)?.to_vec()),
        },
        _ => match v {
            Value::Bool(b) => Param::Bytes(if b { b"1".to_vec() } else { vec![] }),
            other => Param::Bytes(it.to_str(&other)?.to_vec()),
        },
    })
}

/// SQL literal for an emulated prepare.
fn literal(it: &Interp, v: &Value, kind: i64) -> R<Vec<u8>> {
    Ok(match to_param(it, v, kind)? {
        Param::Null => b"NULL".to_vec(),
        Param::Int(n) if kind & 0x7FFF_FFFF == PARAM_INT || kind & 0x7FFF_FFFF == PARAM_BOOL => n.to_string().into_bytes(),
        Param::Int(n) => sql::mysql_quote(n.to_string().as_bytes()),
        Param::Float(f) => sql::mysql_quote(crate::value::fmt_float(f).as_bytes()),
        Param::Bytes(b) => sql::mysql_quote(&b),
    })
}

// ---- PDO methods ----------------------------------------------------------------------------

fn record(db: &mut Db, r: &Result<(), DbError>) {
    match r {
        Ok(()) => {
            db.error = None;
            db.last_ok = true;
        }
        Err(e) => db.error = Some(e.clone()),
    }
}

fn new_statement(it: &Interp, pdo: &Rc<Obj>, db: &Shared, sql_text: Vec<u8>) -> R<Result<Rc<Obj>, DbError>> {
    let pieces = sql::scan(&sql_text);
    let mut names = vec![];
    let mut positional = 0;
    for p in &pieces {
        match p {
            Piece::Named(n) => names.push(n.clone()),
            Piece::Positional => positional += 1,
            _ => {}
        }
    }
    let (emulate, class) = {
        let d = db.borrow();
        (d.attrs.emulate, d.attrs.statement_class.clone())
    };
    if !names.is_empty() && positional > 0 && !emulate {
        return Ok(Err(DbError::new("HY093", 0, "mixed named and positional parameters")));
    }
    let mut server = None;
    if !emulate {
        // Native prepare: placeholders become `?`.
        let mut q = Vec::with_capacity(sql_text.len());
        for p in &pieces {
            match p {
                Piece::Text(a, b) => q.extend_from_slice(&sql_text[*a..*b]),
                _ => q.push(b'?'),
            }
        }
        let r = db.borrow_mut().conn.prepare(&q);
        match r {
            Ok(s) => server = Some(s),
            Err(e) => return Ok(Err(e)),
        }
    }
    let default_fetch = db.borrow().attrs.default_fetch;
    let (cls, ctor_args) = match class {
        Some((c, args)) => (c, args),
        None => (it.lookup_class("PDOStatement")?, vec![]),
    };
    let o = it.new_instance(&cls)?;
    o.set_prop("queryString", Value::bytes(sql_text.clone()));
    let st = StmtState {
        db: db.clone(),
        _pdo: pdo.clone(),
        sql: sql_text,
        pieces,
        names,
        positional,
        server,
        bound: HashMap::new(),
        result: None,
        cursor: 0,
        row_count: 0,
        fetch: FetchMode::plain(default_fetch),
        error: None,
        executed: false,
    };
    o.set_internal(Internal::Data(Box::new(StmtData(Rc::new(RefCell::new(st))))));
    if let Some(ctor) = cls.ctor.clone() {
        if !std::ptr::eq(&*cls, &*it.lookup_class("PDOStatement")?) {
            it.invoke_method(Some(o.clone()), &ctor, cls.clone(), ctor_args, vec![])?;
        }
    }
    Ok(Ok(o))
}

fn pdo_prepare(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let q = it.to_str(&arg(&a, 0))?.to_vec();
    match new_statement(it, this.unwrap(), &db, q)? {
        Ok(o) => {
            record(&mut db.borrow_mut(), &Ok(()));
            Ok(Value::Object(o))
        }
        Err(e) => {
            let errmode = db.borrow().attrs.errmode;
            record(&mut db.borrow_mut(), &Err(e.clone()));
            fail(it, errmode, "PDO::prepare", &e)
        }
    }
}

fn pdo_exec(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let q = it.to_str(&arg(&a, 0))?;
    if q.is_empty() {
        return Err(it.error("ValueError", "PDO::exec(): Argument #1 ($statement) cannot be empty"));
    }
    let r = {
        let mut d = db.borrow_mut();
        let r = d.conn.query_bytes(&q);
        // exec() discards any extra result sets.
        while d.conn.more_results() {
            let _ = d.conn.next_result(false);
        }
        r
    };
    match r {
        Ok(rs) => {
            record(&mut db.borrow_mut(), &Ok(()));
            Ok(Value::Int(if rs.columns.is_empty() { rs.affected as i64 } else { rs.rows.len() as i64 }))
        }
        Err(e) => {
            let errmode = db.borrow().attrs.errmode;
            record(&mut db.borrow_mut(), &Err(e.clone()));
            fail(it, errmode, "PDO::exec", &e)
        }
    }
}

fn pdo_query(it: &Interp, this: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let v = pdo_prepare(it, this, cls, vec![arg(&a, 0)])?;
    let Value::Object(st) = v else { return Ok(v) };
    let ok = stmt_execute(it, Some(&st), cls, vec![])?;
    if !ok.truthy() {
        // Mirror the statement's error on the connection.
        let s = stmt_of(it, Some(&st))?;
        let e = s.borrow().error.clone();
        let db = pdo_of(it, this)?;
        db.borrow_mut().error = e;
        return Ok(Value::Bool(false));
    }
    if a.len() > 1 && !arg(&a, 1).is_null() {
        stmt_set_fetch_mode(it, Some(&st), cls, a[1..].to_vec())?;
    }
    Ok(Value::Object(st))
}

fn pdo_last_insert_id(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let id = db.borrow().conn.insert_id;
    Ok(Value::str(&id.to_string()))
}

fn tx(it: &Interp, this: Option<&Rc<Obj>>, sql: &str, want: Option<bool>, fname: &str) -> R<Value> {
    let db = pdo_of(it, this)?;
    let in_tx = db.borrow().in_tx || db.borrow().conn.in_transaction();
    match want {
        Some(false) if in_tx => return Err(it.error("PDOException", "There is already an active transaction")),
        Some(true) if !in_tx => return Err(it.error("PDOException", "There is no active transaction")),
        _ => {}
    }
    let r = db.borrow_mut().conn.query(sql).map(|_| ());
    match r {
        Ok(()) => {
            let mut d = db.borrow_mut();
            d.in_tx = sql.starts_with("START");
            record(&mut d, &Ok(()));
            Ok(Value::Bool(true))
        }
        Err(e) => {
            let errmode = db.borrow().attrs.errmode;
            db.borrow_mut().in_tx = false;
            record(&mut db.borrow_mut(), &Err(e.clone()));
            fail(it, errmode, fname, &e)
        }
    }
}

fn pdo_begin(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    tx(it, this, "START TRANSACTION", Some(false), "PDO::beginTransaction")
}

fn pdo_commit(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    tx(it, this, "COMMIT", Some(true), "PDO::commit")
}

fn pdo_rollback(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    tx(it, this, "ROLLBACK", Some(true), "PDO::rollBack")
}

fn pdo_in_transaction(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let d = db.borrow();
    Ok(Value::Bool(d.in_tx || d.conn.in_transaction()))
}

fn pdo_quote(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    pdo_of(it, this)?;
    let s = it.to_str(&arg(&a, 0))?;
    Ok(Value::bytes(sql::mysql_quote(&s)))
}

fn error_info(e: &Option<DbError>, any_op: bool) -> Value {
    match e {
        Some(e) => Value::array(PhpArray::from_list([
            Value::str(&e.sqlstate),
            if e.code == 0 { Value::Null } else { Value::Int(e.code) },
            Value::str(&e.message),
        ])),
        None => Value::array(PhpArray::from_list([if any_op { Value::str("00000") } else { Value::str("") }, Value::Null, Value::Null])),
    }
}

fn pdo_error_code(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let d = db.borrow();
    Ok(match &d.error {
        Some(e) => Value::str(&e.sqlstate),
        None if d.last_ok => Value::str("00000"),
        None => Value::Null,
    })
}

fn pdo_error_info(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let d = db.borrow();
    Ok(error_info(&d.error, d.last_ok))
}

fn pdo_get_attribute(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let d = db.borrow();
    let at = &d.attrs;
    Ok(match int_arg(it, &a, 0, -1)? {
        ATTR_ERRMODE => Value::Int(at.errmode),
        ATTR_CASE => Value::Int(at.case),
        ATTR_ORACLE_NULLS => Value::Int(at.oracle_nulls),
        ATTR_STRINGIFY_FETCHES => Value::Bool(at.stringify),
        ATTR_EMULATE_PREPARES => Value::Bool(at.emulate),
        ATTR_DEFAULT_FETCH_MODE => Value::Int(at.default_fetch),
        ATTR_PERSISTENT => Value::Bool(at.persistent),
        ATTR_AUTOCOMMIT => Value::Bool(at.autocommit),
        ATTR_TIMEOUT => Value::Int(at.timeout),
        ATTR_DEFAULT_STR_PARAM => Value::Int(at.default_str),
        ATTR_DRIVER_NAME => Value::str(d.driver),
        ATTR_SERVER_VERSION => Value::str(&d.conn.server_version),
        ATTR_CLIENT_VERSION => Value::str("mysqlnd 8.5.10"),
        ATTR_CONNECTION_STATUS => Value::str(d.conn.host_info()),
        ATTR_SERVER_INFO => Value::str(&format!("Uptime: 0  Threads: 1  Questions: 0  Slow queries: 0  Opens: 0  Flush tables: 0  Open tables: 0  Queries per second avg: 0.000")),
        ATTR_STATEMENT_CLASS => match &at.statement_class {
            Some((c, args)) => Value::array(PhpArray::from_list([Value::str(&c.name), Value::array(PhpArray::from_list(args.iter().cloned()))])),
            None => Value::array(PhpArray::from_list([Value::str("PDOStatement")])),
        },
        MYSQL_ATTR_USE_BUFFERED_QUERY => Value::Bool(at.buffered),
        _ => {
            drop(d);
            let errmode = db.borrow().attrs.errmode;
            return fail(it, errmode, "PDO::getAttribute", &DbError::new("IM001", 0, "driver does not support that attribute"));
        }
    })
}

fn pdo_set_attribute(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let db = pdo_of(it, this)?;
    let k = int_arg(it, &a, 0, -1)?;
    let v = arg(&a, 1);
    let mut d = db.borrow_mut();
    match k {
        ATTR_ERRMODE => {
            let m = it.to_int(&v)?;
            if !(0..=2).contains(&m) {
                return Err(it.error("ValueError", "PDO::setAttribute(): Argument #2 ($value) must be one of PDO::ERRMODE_SILENT, PDO::ERRMODE_WARNING, or PDO::ERRMODE_EXCEPTION"));
            }
            d.attrs.errmode = m;
        }
        ATTR_CASE => d.attrs.case = it.to_int(&v)?,
        ATTR_ORACLE_NULLS => d.attrs.oracle_nulls = it.to_int(&v)?,
        ATTR_STRINGIFY_FETCHES => d.attrs.stringify = v.truthy(),
        ATTR_EMULATE_PREPARES => d.attrs.emulate = v.truthy(),
        ATTR_DEFAULT_FETCH_MODE => d.attrs.default_fetch = it.to_int(&v)?,
        ATTR_AUTOCOMMIT => {
            d.attrs.autocommit = v.truthy();
            let q = if v.truthy() { "SET autocommit=1" } else { "SET autocommit=0" };
            let _ = d.conn.query(q);
        }
        ATTR_TIMEOUT => d.attrs.timeout = it.to_int(&v)?,
        ATTR_DEFAULT_STR_PARAM => d.attrs.default_str = it.to_int(&v)?,
        MYSQL_ATTR_USE_BUFFERED_QUERY => d.attrs.buffered = v.truthy(),
        ATTR_STATEMENT_CLASS => {
            let Value::Array(arr) = v else { return Ok(Value::Bool(false)) };
            let name = arr.get(&Key::Int(0)).map(|n| it.to_str(&n)).transpose()?.map(|n| String::from_utf8_lossy(&n).into_owned()).unwrap_or_default();
            let cls = it.lookup_class(&name)?;
            let args = match arr.get(&Key::Int(1)) {
                Some(Value::Array(x)) => x.values().collect(),
                _ => vec![],
            };
            d.attrs.statement_class = if cls.lname.as_ref() == "pdostatement" { None } else { Some((cls, args)) };
        }
        ATTR_PERSISTENT => d.attrs.persistent = v.truthy(),
        _ => {}
    }
    Ok(Value::Bool(true))
}

fn pdo_drivers(_: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::array(PhpArray::from_list([Value::str("mysql")])))
}

// ---- PDOStatement ---------------------------------------------------------------------------

fn pkey(it: &Interp, v: &Value) -> R<PKey> {
    Ok(match v {
        Value::Int(n) => PKey::Pos(*n as usize),
        other => {
            let s = String::from_utf8_lossy(&it.to_str(other)?).into_owned();
            PKey::Name(if s.starts_with(':') { s } else { format!(":{s}") })
        }
    })
}

fn stmt_bind_value(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let k = pkey(it, &arg(&a, 0))?;
    let kind = int_arg(it, &a, 2, PARAM_STR)?;
    let mut s = st.borrow_mut();
    if let PKey::Pos(0) = k {
        let e = DbError::new("HY093", 0, "Columns/Parameters are 1-based");
        let errmode = s.db.borrow().attrs.errmode;
        s.error = Some(e.clone());
        drop(s);
        return fail(it, errmode, "PDOStatement::bindValue", &e);
    }
    if let PKey::Name(n) = &k {
        if !s.names.contains(n) && s.server.is_some() {
            let e = DbError::new("HY093", 0, "parameter was not defined");
            let errmode = s.db.borrow().attrs.errmode;
            s.error = Some(e.clone());
            drop(s);
            return fail(it, errmode, "PDOStatement::bindValue", &e);
        }
    }
    s.bound.insert(k, Bound { value: arg(&a, 1).deref(), kind });
    Ok(Value::Bool(true))
}

/// Parameters in placeholder order, or an HY093 error.
fn ordered_params(s: &StmtState) -> Result<Vec<Bound>, DbError> {
    let mut out = vec![];
    let mismatch = || DbError::new("HY093", 0, if s.server.is_some() { "" } else { "number of bound variables does not match number of tokens" });
    if !s.names.is_empty() && s.positional > 0 {
        return Err(DbError::new("HY093", 0, "mixed named and positional parameters"));
    }
    if s.names.is_empty() {
        for i in 1..=s.positional {
            match s.bound.get(&PKey::Pos(i)) {
                Some(b) => out.push(b.clone()),
                None => return Err(mismatch()),
            }
        }
        if s.bound.len() > s.positional {
            return Err(mismatch());
        }
    } else {
        for n in &s.names {
            match s.bound.get(&PKey::Name(n.clone())) {
                Some(b) => out.push(b.clone()),
                None => return Err(DbError::new("HY093", 0, "parameter was not defined")),
            }
        }
    }
    Ok(out)
}

fn stmt_execute(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    // execute([...]) binds every value as a string (keys: 0-based list or names).
    if let Value::Array(params) = arg(&a, 0) {
        let mut s = st.borrow_mut();
        s.bound.clear();
        for (k, v) in params.iter() {
            let key = match k {
                Key::Int(n) => PKey::Pos(n as usize + 1),
                Key::Str(n) => {
                    let n = String::from_utf8_lossy(&n).into_owned();
                    PKey::Name(if n.starts_with(':') { n } else { format!(":{n}") })
                }
            };
            s.bound.insert(key, Bound { value: v.deref(), kind: PARAM_STR });
        }
    }
    // bindParam(): references kept by the PHP side; bind their values now.
    if let Some(Value::Array(refs)) = this.and_then(|o| o.get_prop("__phpx_refs")) {
        let types = match this.and_then(|o| o.get_prop("__phpx_types")) {
            Some(Value::Array(t)) => Some(t),
            _ => None,
        };
        let mut s = st.borrow_mut();
        for (k, v) in refs.iter() {
            let kind = types.as_ref().and_then(|t| t.get(&k)).map(|t| it.to_int(&t)).transpose()?.unwrap_or(PARAM_STR);
            let key = match &k {
                Key::Int(n) => PKey::Pos(*n as usize),
                Key::Str(n) => {
                    let n = String::from_utf8_lossy(n).into_owned();
                    PKey::Name(if n.starts_with(':') { n } else { format!(":{n}") })
                }
            };
            s.bound.insert(key, Bound { value: v.deref(), kind });
        }
    }
    let (db, errmode) = {
        let s = st.borrow();
        let errmode = s.db.borrow().attrs.errmode;
        (s.db.clone(), errmode)
    };
    let r: Result<ResultSet, DbError> = (|| {
        let mut s = st.borrow_mut();
        if s.server.is_none() && s.bound.is_empty() {
            // Emulated prepare with nothing bound: the SQL goes as written.
            s.result = None;
            s.cursor = 0;
            let q = s.sql.clone();
            drop(s);
            let r = db.borrow_mut().conn.query_bytes(&q);
            return r;
        }
        let params = ordered_params(&s)?;
        s.result = None;
        s.cursor = 0;
        if let Some(server) = &s.server {
            let ps = params.iter().map(|b| to_param(it, &b.value, b.kind)).collect::<R<Vec<_>>>().map_err(|_| DbError::new("HY000", 0, "invalid parameter value"))?;
            let r = db.borrow_mut().conn.execute(server, &ps);
            return r;
        }
        // Emulated: substitute literals.
        let mut q = Vec::with_capacity(s.sql.len() + 32);
        let mut i = 0;
        for p in &s.pieces {
            match p {
                Piece::Text(a, b) => q.extend_from_slice(&s.sql[*a..*b]),
                _ => {
                    let b = &params[i];
                    q.extend(literal(it, &b.value, b.kind).map_err(|_| DbError::new("HY000", 0, "invalid parameter value"))?);
                    i += 1;
                }
            }
        }
        drop(s);
        let r = db.borrow_mut().conn.query_bytes(&q);
        r
    })();
    match r {
        Ok(rs) => {
            let mut s = st.borrow_mut();
            s.row_count = if rs.columns.is_empty() { rs.affected as i64 } else { rs.rows.len() as i64 };
            s.result = Some(rs);
            s.error = None;
            s.executed = true;
            let mut d = db.borrow_mut();
            d.error = None;
            d.last_ok = true;
            Ok(Value::Bool(true))
        }
        Err(e) => {
            st.borrow_mut().error = Some(e.clone());
            db.borrow_mut().error = Some(e.clone());
            fail(it, errmode, "PDOStatement::execute", &e)
        }
    }
}

/// Build one row for `mode` from the current result row.
fn make_row(it: &Interp, s: &StmtState, row: &[Cell], fm: &FetchMode) -> R<Value> {
    let rs = s.result.as_ref().unwrap();
    let attrs = s.db.borrow().attrs.clone();
    let cols = &rs.columns;
    let vals = || row.iter().zip(cols).map(|(c, col)| to_php(it, c, col, &attrs));
    let mode = fm.mode & !FETCH_FLAGS;
    Ok(match mode {
        FETCH_ASSOC => {
            let mut a = PhpArray::new();
            for (col, v) in cols.iter().zip(vals()) {
                a.insert(Key::Str(pstr(&column_name(col, &attrs))), v);
            }
            Value::array(a)
        }
        FETCH_NUM => Value::array(PhpArray::from_list(vals())),
        FETCH_BOTH | FETCH_DEFAULT => {
            let mut a = PhpArray::new();
            for (i, (col, v)) in cols.iter().zip(vals()).enumerate() {
                a.insert(Key::Str(pstr(&column_name(col, &attrs))), v.clone());
                a.insert(Key::Int(i as i64), v);
            }
            Value::array(a)
        }
        FETCH_NAMED => {
            let mut a = PhpArray::new();
            for (col, v) in cols.iter().zip(vals()) {
                let k = Key::Str(pstr(&column_name(col, &attrs)));
                match a.get(&k) {
                    Some(Value::Array(mut prev)) => {
                        std::rc::Rc::make_mut(&mut prev).push(v);
                        a.insert(k, Value::Array(prev));
                    }
                    Some(prev) => a.insert(k, Value::array(PhpArray::from_list([prev, v]))),
                    None => a.insert(k, v),
                }
            }
            Value::array(a)
        }
        FETCH_OBJ | FETCH_LAZY => {
            let o = Obj::new(it.lookup_class("stdClass")?, Internal::None);
            for (col, v) in cols.iter().zip(vals()) {
                o.set_prop(&column_name(col, &attrs), v);
            }
            Value::Object(o)
        }
        FETCH_COLUMN => match row.get(fm.column) {
            Some(c) => to_php(it, c, &cols[fm.column], &attrs),
            None => return Err(it.error("ValueError", "Invalid column index")),
        },
        FETCH_CLASS | FETCH_INTO => {
            let (cls, mut values): (Rc<Class>, Vec<(String, Value)>) = (
                fm.class.clone().unwrap_or(it.lookup_class("stdClass")?),
                cols.iter().zip(vals()).map(|(c, v)| (column_name(c, &attrs), v)).collect(),
            );
            let (cls, skip_first) = if fm.mode & FETCH_CLASSTYPE != 0 && !values.is_empty() {
                let name = String::from_utf8_lossy(&it.to_str(&values[0].1)?).into_owned();
                (it.find_class(&name)?.unwrap_or(cls), true)
            } else {
                (cls, false)
            };
            if skip_first {
                values.remove(0);
            }
            let o = match (&fm.into, mode) {
                (Some(o), FETCH_INTO) => o.clone(),
                _ => it.new_instance(&cls)?,
            };
            let mut scope = Frame::new(cls.decl_ctx.clone(), 0);
            scope.class = Some(cls.clone());
            let late = fm.mode & FETCH_PROPS_LATE != 0;
            let run_ctor = |it: &Interp| -> R<()> {
                if mode == FETCH_CLASS {
                    if let Some(ctor) = cls.ctor.clone() {
                        it.invoke_method(Some(o.clone()), &ctor, cls.clone(), fm.ctor_args.clone(), vec![])?;
                    }
                }
                Ok(())
            };
            if late {
                run_ctor(it)?;
            }
            for (k, v) in values {
                it.set_prop(&scope, &o, &k, v)?;
            }
            if !late {
                run_ctor(it)?;
            }
            Value::Object(o)
        }
        _ => return Err(it.error("ValueError", "PDOStatement::fetch(): Argument #1 ($mode) must be a bitmask of PDO::FETCH_* constants")),
    })
}

fn mode_from_args(it: &Interp, s: &StmtState, a: &[Value], start: usize) -> R<FetchMode> {
    let mode = int_arg(it, a, start, FETCH_DEFAULT)?;
    if mode == FETCH_DEFAULT {
        return Ok(s.fetch.clone());
    }
    let mut fm = FetchMode::plain(mode);
    match mode & !FETCH_FLAGS {
        FETCH_COLUMN => fm.column = int_arg(it, a, start + 1, 0)?.max(0) as usize,
        FETCH_CLASS => {
            if mode & FETCH_CLASSTYPE == 0 {
                let name = if a.len() > start + 1 { str_arg(it, a, start + 1)? } else { "stdClass".into() };
                fm.class = Some(it.lookup_class(&name)?);
            }
            if let Value::Array(x) = arg(a, start + 2) {
                fm.ctor_args = x.values().collect();
            }
        }
        FETCH_INTO => {
            if let Value::Object(o) = arg(a, start + 1) {
                fm.into = Some(o.clone());
                fm.class = Some(o.class.clone());
            }
        }
        _ => {}
    }
    Ok(fm)
}

fn stmt_fetch(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let s = st.borrow();
    let fm = mode_from_args(it, &s, &a, 0)?;
    let Some(rs) = &s.result else { return Ok(Value::Bool(false)) };
    let Some(row) = rs.rows.get(s.cursor) else { return Ok(Value::Bool(false)) };
    let row = row.clone();
    if fm.mode & !FETCH_FLAGS == FETCH_BOUND {
        let names: Vec<String> = rs.columns.iter().map(|c| c.name.clone()).collect();
        let attrs = s.db.borrow().attrs.clone();
        let vals: Vec<Value> = row.iter().zip(&rs.columns).map(|(c, col)| to_php(it, c, col, &attrs)).collect();
        drop(s);
        st.borrow_mut().cursor += 1;
        let mut arr = PhpArray::new();
        for (i, (n, v)) in names.iter().zip(vals).enumerate() {
            arr.insert(Key::Str(pstr(n)), v.clone());
            arr.insert(Key::Int(i as i64 + 1), v);
        }
        it.call_method(this.unwrap(), "__phpx_assignBound", vec![Value::array(arr)])?;
        return Ok(Value::Bool(true));
    }
    let v = make_row(it, &s, &row, &fm)?;
    drop(s);
    st.borrow_mut().cursor += 1;
    Ok(v)
}

fn stmt_fetch_all(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let s = st.borrow();
    let fm = mode_from_args(it, &s, &a, 0)?;
    let Some(rs) = &s.result else { return Ok(Value::array(PhpArray::new())) };
    let rows: Vec<Vec<Cell>> = rs.rows[s.cursor.min(rs.rows.len())..].to_vec();
    let ncols = rs.columns.len();
    let base = fm.mode & !FETCH_FLAGS;
    let group = fm.mode & FETCH_GROUP != 0;
    let unique = fm.mode & FETCH_UNIQUE != 0;
    let mut out = PhpArray::new();
    let attrs = s.db.borrow().attrs.clone();
    let cols = rs.columns.clone();
    if base == FETCH_FUNC {
        let f = it.to_callable(&Frame::new(it.file_ctx(0), 0), &arg(&a, 1))?;
        drop(s);
        for row in rows {
            let vals: Vec<Value> = row.iter().zip(&cols).map(|(c, col)| to_php(it, c, col, &attrs)).collect();
            out.push(it.call_callable(&f, vals, vec![])?);
        }
        st.borrow_mut().cursor += out.len();
        return Ok(Value::array(out));
    }
    if base == FETCH_KEY_PAIR {
        if ncols != 2 {
            let e = DbError::new("HY000", 0, "General error: PDO::FETCH_KEY_PAIR fetch mode requires the result set to contain exactly 2 columns.");
            return Err(it.error("PDOException", &format!("SQLSTATE[HY000]: {}", e.message)));
        }
        for row in &rows {
            let k = to_php(it, &row[0], &cols[0], &attrs);
            let v = to_php(it, &row[1], &cols[1], &attrs);
            out.insert(it.to_key(&k)?, v);
        }
    } else if group || unique {
        for row in &rows {
            let k = match to_php(it, &row[0], &cols[0], &attrs) {
                Value::Null => Key::Str(pstr("")),
                v => it.to_key(&v)?,
            };
            let rest: Vec<Cell> = row[1..].to_vec();
            let sub = shallow(&s, cols[1..].to_vec());
            let mut sfm = fm.clone();
            sfm.mode = base;
            if base == FETCH_COLUMN {
                sfm.column = 0;
            }
            let v = make_row(it, &sub, &rest, &sfm)?;
            if unique {
                out.insert(k, v);
            } else {
                let mut list = match out.get(&k) {
                    Some(Value::Array(l)) => (*l).clone(),
                    _ => PhpArray::new(),
                };
                list.push(v);
                out.insert(k, Value::array(list));
            }
        }
    } else {
        for row in &rows {
            out.push(make_row(it, &s, row, &fm)?);
        }
    }
    let n = rows.len();
    drop(s);
    st.borrow_mut().cursor += n;
    Ok(Value::array(out))
}

/// A statement view over a subset of columns (FETCH_GROUP / UNIQUE).
fn shallow(s: &StmtState, cols: Vec<Column>) -> StmtState {
    StmtState {
        db: s.db.clone(),
        _pdo: s._pdo.clone(),
        sql: vec![],
        pieces: vec![],
        names: vec![],
        positional: 0,
        server: None,
        bound: HashMap::new(),
        result: Some(ResultSet { columns: cols, ..Default::default() }),
        cursor: 0,
        row_count: 0,
        fetch: s.fetch.clone(),
        error: None,
        executed: true,
    }
}

fn stmt_fetch_column(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let col = int_arg(it, &a, 0, 0)?;
    let s = st.borrow();
    let Some(rs) = &s.result else { return Ok(Value::Bool(false)) };
    let Some(row) = rs.rows.get(s.cursor) else { return Ok(Value::Bool(false)) };
    if col < 0 || col as usize >= rs.columns.len() {
        return Err(it.error("ValueError", "PDOStatement::fetchColumn(): Argument #1 ($column) must be greater than or equal to 0"));
    }
    let attrs = s.db.borrow().attrs.clone();
    let v = to_php(it, &row[col as usize], &rs.columns[col as usize], &attrs);
    drop(s);
    st.borrow_mut().cursor += 1;
    Ok(v)
}

fn stmt_fetch_object(it: &Interp, this: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let class = if a.is_empty() || arg(&a, 0).is_null() { Value::str("stdClass") } else { arg(&a, 0) };
    stmt_fetch(it, this, cls, vec![Value::Int(FETCH_CLASS), class, arg(&a, 1)]).and_then(|v| {
        // fetch() with a class: take the mode arguments directly.
        Ok(v)
    })
}

fn stmt_row_count(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Int(stmt_of(it, this)?.borrow().row_count))
}

fn stmt_column_count(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let s = st.borrow();
    Ok(Value::Int(match (&s.result, &s.server) {
        (Some(r), _) => r.columns.len() as i64,
        (None, Some(sv)) => sv.columns.len() as i64,
        _ => 0,
    }))
}

fn stmt_close_cursor(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let mut s = st.borrow_mut();
    s.result = None;
    s.cursor = 0;
    Ok(Value::Bool(true))
}

fn stmt_next_rowset(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let (db, binary) = {
        let s = st.borrow();
        (s.db.clone(), s.server.is_some())
    };
    let r = db.borrow_mut().conn.next_result(binary);
    match r {
        Ok(Some(rs)) => {
            let mut s = st.borrow_mut();
            s.row_count = if rs.columns.is_empty() { rs.affected as i64 } else { rs.rows.len() as i64 };
            s.result = Some(rs);
            s.cursor = 0;
            Ok(Value::Bool(true))
        }
        Ok(None) => Ok(Value::Bool(false)),
        Err(e) => {
            let errmode = db.borrow().attrs.errmode;
            st.borrow_mut().error = Some(e.clone());
            fail(it, errmode, "PDOStatement::nextRowset", &e)
        }
    }
}

fn stmt_set_fetch_mode(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let fm = {
        let s = st.borrow();
        mode_from_args(it, &s, &a, 0)?
    };
    st.borrow_mut().fetch = fm;
    Ok(Value::Bool(true))
}

fn stmt_error_code(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let s = st.borrow();
    Ok(match &s.error {
        Some(e) => Value::str(&e.sqlstate),
        None if s.executed => Value::str("00000"),
        None => Value::Null,
    })
}

fn stmt_error_info(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let s = st.borrow();
    Ok(error_info(&s.error, s.executed))
}

fn stmt_column_meta(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let i = int_arg(it, &a, 0, 0)?;
    if i < 0 {
        return Err(it.error("ValueError", "PDOStatement::getColumnMeta(): Argument #1 ($column) must be greater than or equal to 0"));
    }
    let s = st.borrow();
    let cols = match (&s.result, &s.server) {
        (Some(r), _) => &r.columns,
        (None, Some(sv)) => &sv.columns,
        _ => return Ok(Value::Bool(false)),
    };
    let Some(c) = cols.get(i as usize) else { return Ok(Value::Bool(false)) };
    let native = match c.kind {
        mysql::TYPE_TINY => "TINY",
        mysql::TYPE_SHORT => "SHORT",
        mysql::TYPE_LONG => "LONG",
        mysql::TYPE_INT24 => "INT24",
        mysql::TYPE_LONGLONG => "LONGLONG",
        mysql::TYPE_FLOAT => "FLOAT",
        mysql::TYPE_DOUBLE => "DOUBLE",
        mysql::TYPE_NEWDECIMAL | mysql::TYPE_DECIMAL => "NEWDECIMAL",
        mysql::TYPE_DATE => "DATE",
        mysql::TYPE_TIME => "TIME",
        mysql::TYPE_DATETIME => "DATETIME",
        mysql::TYPE_TIMESTAMP => "TIMESTAMP",
        mysql::TYPE_YEAR => "YEAR",
        mysql::TYPE_VAR_STRING => "VAR_STRING",
        mysql::TYPE_STRING => "STRING",
        252 => "BLOB",
        245 => "JSON",
        16 => "BIT",
        _ => "STRING",
    };
    let pdo_type = match c.kind {
        mysql::TYPE_TINY | mysql::TYPE_SHORT | mysql::TYPE_LONG | mysql::TYPE_INT24 | mysql::TYPE_LONGLONG | mysql::TYPE_YEAR => PARAM_INT,
        _ => PARAM_STR,
    };
    let mut flags = vec![];
    if c.flags & 1 != 0 {
        flags.push(Value::str("not_null"));
    }
    if c.flags & 2 != 0 {
        flags.push(Value::str("primary_key"));
    }
    if c.flags & 4 != 0 {
        flags.push(Value::str("unique_key"));
    }
    if c.flags & 8 != 0 {
        flags.push(Value::str("multiple_key"));
    }
    if c.flags & 16 != 0 {
        flags.push(Value::str("blob"));
    }
    let mut m = PhpArray::new();
    for (k, v) in [
        ("native_type", Value::str(native)),
        ("pdo_type", Value::Int(pdo_type)),
        ("flags", Value::array(PhpArray::from_list(flags))),
        ("table", Value::str(&c.table)),
        ("name", Value::str(&c.name)),
        ("len", Value::Int(c.length as i64)),
        ("precision", Value::Int(c.decimals as i64)),
    ] {
        m.insert(Key::Str(pstr(k)), v);
    }
    Ok(Value::array(m))
}

fn stmt_debug_dump(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let st = stmt_of(it, this)?;
    let s = st.borrow();
    let mut out = format!("SQL: [{}] {}\nParams:  {}\n", s.sql.len(), String::from_utf8_lossy(&s.sql), s.bound.len());
    let mut keys: Vec<&PKey> = s.bound.keys().collect();
    keys.sort_by_key(|k| format!("{k:?}"));
    for k in keys {
        let b = &s.bound[k];
        match k {
            PKey::Pos(n) => out.push_str(&format!("Key: Position #{}:\nparamno={}\nname=[0] \"\"\nis_param=1\nparam_type={}\n", n - 1, n - 1, b.kind)),
            PKey::Name(n) => out.push_str(&format!("Key: Name: [{}] {}\nparamno=-1\nname=[{}] \"{}\"\nis_param=1\nparam_type={}\n", n.len(), n, n.len(), n, b.kind)),
        }
    }
    it.echo(out.as_bytes());
    Ok(Value::Null)
}

fn stmt_get_attribute(_: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Bool(false))
}

fn stmt_set_attribute(_: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Bool(false))
}


pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("pdo", "__construct", pdo_construct as NativeMethod),
        ("pdo", "prepare", pdo_prepare),
        ("pdo", "exec", pdo_exec),
        ("pdo", "query", pdo_query),
        ("pdo", "lastinsertid", pdo_last_insert_id),
        ("pdo", "begintransaction", pdo_begin),
        ("pdo", "commit", pdo_commit),
        ("pdo", "rollback", pdo_rollback),
        ("pdo", "intransaction", pdo_in_transaction),
        ("pdo", "quote", pdo_quote),
        ("pdo", "errorcode", pdo_error_code),
        ("pdo", "errorinfo", pdo_error_info),
        ("pdo", "getattribute", pdo_get_attribute),
        ("pdo", "setattribute", pdo_set_attribute),
        ("pdo", "static getavailabledrivers", pdo_drivers),
        ("pdostatement", "bindvalue", stmt_bind_value),
        ("pdostatement", "execute", stmt_execute),
        ("pdostatement", "fetch", stmt_fetch),
        ("pdostatement", "fetchall", stmt_fetch_all),
        ("pdostatement", "fetchcolumn", stmt_fetch_column),
        ("pdostatement", "fetchobject", stmt_fetch_object),
        ("pdostatement", "rowcount", stmt_row_count),
        ("pdostatement", "columncount", stmt_column_count),
        ("pdostatement", "closecursor", stmt_close_cursor),
        ("pdostatement", "nextrowset", stmt_next_rowset),
        ("pdostatement", "setfetchmode", stmt_set_fetch_mode),
        ("pdostatement", "errorcode", stmt_error_code),
        ("pdostatement", "errorinfo", stmt_error_info),
        ("pdostatement", "getcolumnmeta", stmt_column_meta),
        ("pdostatement", "debugdumpparams", stmt_debug_dump),
        ("pdostatement", "getattribute", stmt_get_attribute),
        ("pdostatement", "setattribute", stmt_set_attribute),
    ]
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("pdo_drivers", |_, _, _| Ok(Value::array(PhpArray::from_list([Value::str("mysql")]))));
    let _ = Callable::Builtin;
}
