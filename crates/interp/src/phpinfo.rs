//! phpinfo(): PHP's page, laid out the way ext/standard/info.c prints it
//! (text for the CLI, HTML otherwise), from the module sections and
//! directive table PHP 8.5.10 itself produced (phpinfo_data.rs, generated
//! by tools/phpinfo/gen.py) and this process's live configuration.

use crate::interp::{Frame, Interp, R};
use crate::phpinfo_data::{CREDITS_HTML, CREDITS_TEXT, INI, LICENSE_HTML, LICENSE_TEXT, MODULES, PHP_LOGO, STYLE, ZEND_LOGO};
use crate::value::Value;

/// How phpinfo() shows a directive's value (PHP's ini displayers).
#[derive(Clone, Copy, PartialEq)]
pub enum Display {
    Plain,
    /// On / Off.
    Bool,
    /// highlight.*: the colour, as a span in that colour.
    Color,
    /// STDOUT / STDERR / On / Off.
    DisplayErrors,
}

/// A php.ini directive: built-in default (None = PHP's NULL, shown as "no
/// value"), access level (PHP_INI_USER 1, PERDIR 2, SYSTEM 4) and module.
pub struct IniDef {
    pub name: &'static str,
    pub value: Option<&'static str>,
    pub access: u8,
    pub module: &'static str,
    pub display: Display,
}

/// One row of a module's phpinfo() section. (Some kinds only occur in
/// modules phpx doesn't implement yet; the generator emits them all.)
#[allow(dead_code)]
pub enum Row {
    Header(&'static [&'static str]),
    HeaderSpan(&'static str),
    Row(&'static [&'static str]),
    Label(&'static str),
    Note(&'static [&'static str]),
    /// The module's directive table, printed from the live values.
    Directives,
}

pub struct ModuleInfo {
    pub name: &'static str,
    pub tables: &'static [&'static [Row]],
}

pub const INFO_GENERAL: i64 = 1;
pub const INFO_CREDITS: i64 = 2;
pub const INFO_CONFIGURATION: i64 = 4;
pub const INFO_MODULES: i64 = 8;
pub const INFO_ENVIRONMENT: i64 = 16;
pub const INFO_VARIABLES: i64 = 32;
pub const INFO_LICENSE: i64 = 64;

/// The directive `name`, if PHP 8.5 has it in a module phpx implements.
pub fn directive(name: &str) -> Option<&'static IniDef> {
    INI.binary_search_by(|d| d.name.cmp(name)).ok().map(|i| &INI[i])
}

/// The module (as PHP names it) `ext` names, compared case-insensitively.
pub fn module_name(ext: &str) -> Option<&'static str> {
    MODULES.iter().map(|m| m.name).find(|m| m.eq_ignore_ascii_case(ext))
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#039;"),
            c => o.push(c),
        }
    }
    o
}

/// zend_html_puts(): <, >, & escaped, tabs as four spaces.
fn html_puts(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '\t' => o.push_str("    "),
            c => o.push(c),
        }
    }
    o
}

struct Page<'a> {
    it: &'a Interp,
    html: bool,
    out: String,
    /// date_default_timezone_get(), for the date module's rows.
    timezone: String,
}

impl Page<'_> {
    fn put(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn table_start(&mut self) {
        self.put(if self.html { "<table>\n" } else { "\n" });
    }

    fn table_end(&mut self) {
        if self.html {
            self.put("</table>\n");
        }
    }

    fn header(&mut self, cells: &[&str]) {
        if self.html {
            self.put("<tr class=\"h\">");
        }
        for (i, c) in cells.iter().enumerate() {
            let c = if c.is_empty() { " " } else { c };
            if self.html {
                self.put("<th>");
                self.put(c);
                self.put("</th>");
            } else {
                self.put(c);
                self.put(if i + 1 < cells.len() { " => " } else { "\n" });
            }
        }
        if self.html {
            self.put("</tr>\n");
        }
    }

    fn header_span(&mut self, text: &str) {
        if self.html {
            self.put(&format!("<tr class=\"h\"><th colspan=\"2\">{text}</th></tr>\n"));
        } else {
            let spaces = 74usize.saturating_sub(text.len()) / 2;
            let pad = " ".repeat(spaces.max(1));
            self.put(&format!("{pad}{text}{pad}\n"));
        }
    }

    /// php_info_print_table_row().
    fn row(&mut self, cells: &[&str]) {
        if self.html {
            self.put("<tr>");
        }
        for (i, c) in cells.iter().enumerate() {
            if self.html {
                self.put(if i == 0 { "<td class=\"e\">" } else { "<td class=\"v\">" });
                if c.is_empty() {
                    self.put("<i>no value</i>");
                } else {
                    let e = esc(c);
                    self.put(&e);
                }
                self.put(" </td>");
            } else {
                if c.is_empty() {
                    self.put(" ");
                } else {
                    self.put(c);
                    if i + 1 < cells.len() {
                        self.put(" => ");
                    }
                }
                if i + 1 == cells.len() {
                    self.put("\n");
                }
            }
        }
        if self.html {
            self.put("</tr>\n");
        }
    }

    fn section(&mut self, name: &str) {
        if self.html {
            self.put(&format!("<h2>{name}</h2>\n"));
        } else {
            self.table_start();
            self.header(&[name]);
            self.table_end();
        }
    }

    fn hr(&mut self) {
        self.put(if self.html { "<hr />\n" } else { "\n\n _______________________________________________________________________\n\n" });
    }

    /// A directive's value as phpinfo() shows it.
    fn shown(&self, d: &IniDef, v: &str) -> String {
        let no_value = if self.html { "<i>no value</i>" } else { "no value" };
        match d.display {
            Display::Bool => {
                let on = matches!(v.to_ascii_lowercase().as_str(), "on" | "yes" | "true") || v.trim().parse::<i64>().is_ok_and(|n| n != 0);
                (if on { "On" } else { "Off" }).into()
            }
            Display::DisplayErrors => {
                let l = v.to_ascii_lowercase();
                let mode = match l.as_str() {
                    "on" | "yes" | "true" | "stdout" => 1,
                    "stderr" => 2,
                    _ => match v.trim().parse::<i64>() {
                        Ok(2) => 2,
                        Ok(n) if n != 0 => 1,
                        _ => 0,
                    },
                };
                let cli = matches!(crate::server::sapi(), "cli" | "cgi");
                match mode {
                    2 if cli => "STDERR".into(),
                    1 if cli => "STDOUT".into(),
                    0 => "Off".into(),
                    _ => "On".into(),
                }
            }
            // (PHP prints the span in text mode too: the displayer checks
            // the engine's html_errors flag, which the CLI never clears.)
            Display::Color if v.is_empty() => no_value.into(),
            Display::Color => format!("<span style=\"color: {v}\">{v}</span>"),
            Display::Plain if v.is_empty() => no_value.into(),
            Display::Plain if self.html => html_puts(v),
            Display::Plain => v.to_string(),
        }
    }

    fn directives(&mut self, module: &str) {
        let mut first = true;
        // Sorted case-insensitively, like zend_ini_sort_entries().
        let mut defs: Vec<&IniDef> = INI.iter().filter(|d| d.module == module).collect();
        defs.sort_by_key(|d| d.name.to_ascii_lowercase());
        for d in defs {
            if first {
                self.table_start();
                self.header(&["Directive", "Local Value", "Master Value"]);
                first = false;
            }
            let (local, master) = {
                let ini = self.it.ini.borrow();
                (ini.get(d.name).unwrap_or("").to_string(), ini.startup_value(d.name).unwrap_or("").to_string())
            };
            let (l, m) = (self.shown(d, &local), self.shown(d, &master));
            if self.html {
                self.put(&format!("<tr><td class=\"e\">{}</td><td class=\"v\">{l}</td><td class=\"v\">{m}</td></tr>\n", d.name));
            } else {
                self.put(&format!("{} => {l} => {m}\n", d.name));
            }
        }
        if !first {
            self.table_end();
        }
    }

    /// A module row's value: live configuration where PHP shows it.
    fn live(&self, v: &'static str) -> String {
        if v == "{timezone}" {
            return self.timezone.clone();
        }
        if let Some(k) = v.strip_prefix("{ini:").and_then(|k| k.strip_suffix('}')) {
            return self.it.ini_get(k).unwrap_or_default();
        }
        v.to_string()
    }

    fn module(&mut self, m: &ModuleInfo) {
        if self.html {
            let l = m.name.to_ascii_lowercase();
            self.put(&format!("<h2><a name=\"module_{l}\" href=\"#module_{l}\">{}</a></h2>\n", m.name));
        } else {
            self.table_start();
            self.header(&[m.name]);
            self.table_end();
        }
        for t in m.tables {
            if let [Row::Directives] = t {
                self.directives(m.name);
                continue;
            }
            self.table_start();
            for r in *t {
                match r {
                    Row::Header(c) => self.header(c),
                    Row::HeaderSpan(c) => self.header_span(c),
                    Row::Row(c) => {
                        let vals: Vec<String> = c.iter().map(|v| self.live(v)).collect();
                        let refs: Vec<&str> = vals.iter().map(String::as_str).collect();
                        self.row(&refs);
                    }
                    Row::Label(c) => {
                        if self.html {
                            let e = esc(c);
                            self.put(&format!("<tr><td class=\"e\">{e} </td></tr>\n"));
                        } else {
                            self.put(&format!("{c}\n"));
                        }
                    }
                    Row::Note(lines) => {
                        if self.html {
                            self.put("<tr class=\"v\"><td>\n");
                            self.put(&lines.iter().map(|l| esc(l)).collect::<Vec<_>>().join("<br />"));
                            self.put("</td></tr>\n");
                        } else {
                            self.put("\n");
                            self.put(&lines.join("\n"));
                        }
                    }
                    Row::Directives => {}
                }
            }
            self.table_end();
        }
    }

    fn general(&mut self, f: &mut Frame) -> R<()> {
        let it = self.it;
        if self.html {
            self.table_start();
            self.put("<tr class=\"h\"><td>\n");
            self.put(&format!("<a href=\"https://www.php.net/\"><img src=\"{PHP_LOGO}\" alt=\"PHP logo\" /></a>"));
            self.put("<h1 class=\"p\">PHP Version 8.5.10</h1>\n");
            self.put("</td></tr>\n");
            self.table_end();
        } else {
            self.row(&["PHP Version", "8.5.10"]);
        }
        let uname = it.to_string_lossy(&(it.builtins["php_uname"])(it, f, &mut [])?)?;
        let sapi_name = match crate::server::sapi() {
            "cli-server" => "Built-in HTTP server",
            _ => "Command Line Interface",
        };
        let loaded = it.ini.borrow().loaded_file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(none)".into());
        let ini_dir = std::env::var("PHPRC").unwrap_or_else(|_| "/usr/local/etc/php".into());
        self.table_start();
        let rows: Vec<(&str, String)> = vec![
            ("System", uname),
            ("Build Date", env!("PHPX_BUILD_DATE").to_string()),
            ("Build System", "Linux".into()),
            ("Build Provider", "phpx".into()),
            ("Server API", sapi_name.into()),
            ("Virtual Directory Support", "disabled".into()),
            ("Configuration File (php.ini) Path", ini_dir),
            ("Loaded Configuration File", loaded),
            ("Scan this dir for additional .ini files", "(none)".into()),
            ("Additional .ini files parsed", "(none)".into()),
            ("PHP API", "20250925".into()),
            ("PHP Extension", "20250925".into()),
            ("Zend Extension", "420250925".into()),
            ("Zend Extension Build", "API420250925,NTS".into()),
            ("PHP Extension Build", "API20250925,NTS".into()),
            ("PHP Integer Size", "64 bits".into()),
            ("Debug Build", if cfg!(debug_assertions) { "yes" } else { "no" }.into()),
            ("Thread Safety", "disabled".into()),
            ("Zend Signal Handling", "enabled".into()),
            ("Zend Memory Manager", "enabled".into()),
            ("Zend Multibyte Support", "provided by mbstring".into()),
            ("Zend Max Execution Timers", "disabled".into()),
            ("IPv6 Support", "enabled".into()),
            ("DTrace Support", "disabled".into()),
        ];
        for (k, v) in &rows {
            self.row(&[k, v]);
        }
        for (name, func) in [("PHP Streams", "stream_get_wrappers"), ("Stream Socket Transports", "stream_get_transports"), ("Stream Filters", "stream_get_filters")] {
            let list = match (it.builtins[func])(it, f, &mut [])? {
                Value::Array(a) => a.values().map(|v| it.to_string_lossy(&v).unwrap_or_default()).collect::<Vec<_>>(),
                _ => vec![],
            };
            if self.html {
                let items: Vec<String> = list.iter().map(|s| esc(s)).collect();
                self.put(&format!("<tr><td class=\"e\">Registered {name}</td><td class=\"v\">{}</td></tr>\n", items.join(", ")));
            } else {
                self.put(&format!("\nRegistered {name} => {}", list.join(", ")));
            }
        }
        self.table_end();
        // The engine box.
        self.table_start();
        if self.html {
            self.put("<tr class=\"v\"><td>\n");
            self.put(&format!("<a href=\"https://www.zend.com/\"><img src=\"{ZEND_LOGO}\" alt=\"Zend logo\" /></a>\n"));
        } else {
            self.put("\n");
        }
        self.put("This program makes use of the Zend Scripting Language Engine:");
        self.put(if self.html { "<br />" } else { "\n" });
        self.put("Zend Engine v4.5.10, Copyright (c) Zend Technologies\n");
        if self.html {
            self.put("</td></tr>\n");
        }
        self.table_end();
        Ok(())
    }

    fn variables(&mut self, f: &mut Frame) -> R<()> {
        let it = self.it;
        self.section("PHP Variables");
        self.table_start();
        self.header(&["Variable", "Value"]);
        for name in ["_REQUEST", "_GET", "_POST", "_FILES", "_COOKIE", "_SERVER", "_ENV"] {
            let Some(Value::Array(a)) = it.with_globals(f, |g| g.get(name)) else { continue };
            for (k, v) in a.iter() {
                let key = match &k {
                    crate::value::Key::Int(n) => n.to_string(),
                    crate::value::Key::Str(s) => crate::pstr::lossy(s).into_owned(),
                };
                let val = match &v {
                    Value::Array(_) => {
                        let mut args = [v.clone(), Value::Bool(true)];
                        let s = (it.builtins["print_r"])(it, f, &mut args)?;
                        let s = it.to_string_lossy(&s)?;
                        if self.html { format!("<pre>{}</pre>", esc(&s)) } else { s }
                    }
                    other => {
                        let s = it.to_string_lossy(other)?;
                        match (self.html, s.is_empty()) {
                            (true, true) => "<i>no value</i>".into(),
                            (true, false) => esc(&s),
                            _ => s,
                        }
                    }
                };
                if self.html {
                    self.put(&format!("<tr><td class=\"e\">${name}['{}']</td><td class=\"v\">{val}</td></tr>\n", esc(&key)));
                } else {
                    self.put(&format!("${name}['{key}'] => {val}\n"));
                }
            }
        }
        self.table_end();
        Ok(())
    }
}

fn page<'a>(it: &'a Interp, f: &mut Frame) -> R<Page<'a>> {
    let tz = (it.builtins["date_default_timezone_get"])(it, f, &mut [])?;
    Ok(Page { it, html: crate::server::sapi() != "cli", out: String::new(), timezone: it.to_string_lossy(&tz)? })
}

/// phpinfo($flags).
pub fn print(it: &Interp, f: &mut Frame, flags: i64) -> R<()> {
    let mut p = page(it, f)?;
    let html = p.html;
    if html {
        p.put("<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"DTD/xhtml1-transitional.dtd\">\n");
        p.put("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head>\n");
        p.put(STYLE);
        p.put("<title>PHP 8.5.10 - phpinfo()</title><meta name=\"ROBOTS\" content=\"NOINDEX,NOFOLLOW,NOARCHIVE\" /></head>\n");
        p.put("<body><div class=\"center\">\n");
    } else {
        p.put("phpinfo()\n");
    }
    if flags & INFO_GENERAL != 0 {
        p.general(f)?;
    }
    if flags & INFO_CONFIGURATION != 0 {
        p.hr();
        if html {
            p.put("<h1>Configuration</h1>\n");
        } else {
            p.section("Configuration");
        }
        if flags & INFO_MODULES == 0 {
            p.section("PHP Core");
            p.directives("Core");
        }
    }
    if flags & INFO_MODULES != 0 {
        for m in MODULES {
            p.module(m);
        }
        p.section("Additional Modules");
        p.table_start();
        p.header(&["Module Name"]);
        p.table_end();
    }
    if flags & INFO_ENVIRONMENT != 0 {
        p.section("Environment");
        p.table_start();
        p.header(&["Variable", "Value"]);
        for (k, v) in std::env::vars_os() {
            p.row(&[&k.to_string_lossy(), &v.to_string_lossy()]);
        }
        p.table_end();
    }
    if flags & INFO_VARIABLES != 0 {
        p.variables(f)?;
    }
    if flags & INFO_CREDITS != 0 {
        p.hr();
        p.put(if html { CREDITS_HTML } else { CREDITS_TEXT });
    }
    if flags & INFO_LICENSE != 0 {
        p.put(if html { LICENSE_HTML } else { LICENSE_TEXT });
    }
    if html {
        p.put("</div></body></html>");
    }
    it.echo(p.out.as_bytes());
    Ok(())
}
