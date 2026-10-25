<?php
// Buffered: session functions need headers unsent.
ob_start();
$dir = sys_get_temp_dir() . '/phpx_sess_' . getmypid();
@mkdir($dir);
session_save_path($dir);
var_dump(session_status() === PHP_SESSION_NONE, session_name(), session_id());
var_dump(session_start(), session_status() === PHP_SESSION_ACTIVE, strlen(session_id()) >= 22);
$id = session_id();
$_SESSION['count'] = 1;
$_SESSION['list'] = [1, 'two', ['three' => 3.5]];
$_SESSION['obj'] = (object) ['p' => true];
var_dump(session_encode());
var_dump(session_write_close(), session_status() === PHP_SESSION_NONE);
var_dump(file_get_contents("$dir/sess_$id"));
$_SESSION = [];
session_id($id);
var_dump(session_start(), $_SESSION['count'], $_SESSION['list'][2]['three'], $_SESSION['obj']->p);
$_SESSION['count']++;
var_dump(session_decode('extra|s:3:"yes";'), $_SESSION['extra']);
var_dump(session_regenerate_id(true), session_id() !== $id, file_exists("$dir/sess_$id"));
$id2 = session_id();
session_commit();
var_dump(unserialize('a:0:{}'), str_contains(file_get_contents("$dir/sess_$id2"), 'count|i:2;'));
session_start();
var_dump(session_unset(), $_SESSION, session_destroy(), file_exists("$dir/sess_$id2"), session_status() === PHP_SESSION_NONE);
var_dump(session_get_cookie_params());
session_set_cookie_params(['lifetime' => 60, 'path' => '/app', 'httponly' => true, 'samesite' => 'Lax']);
var_dump(session_get_cookie_params()['path'], session_cache_limiter(), session_cache_expire(), session_module_name());

class MemHandler implements SessionHandlerInterface
{
    public static array $store = [];
    public function open(string $path, string $name): bool { echo "open $name\n"; return true; }
    public function close(): bool { echo "close\n"; return true; }
    public function read(string $id): string|false { echo "read\n"; return self::$store[$id] ?? ''; }
    public function write(string $id, string $data): bool { echo "write $data\n"; self::$store[$id] = $data; return true; }
    public function destroy(string $id): bool { unset(self::$store[$id]); return true; }
    public function gc(int $max): int|false { return 0; }
}
var_dump(session_set_save_handler(new MemHandler(), false));
session_id('fixedid123');
session_start();
$_SESSION['user'] = 'alice';
session_write_close();
var_dump(MemHandler::$store);
ini_set('session.serialize_handler', 'php_serialize');
session_id('fixedid123');
session_start();
var_dump($_SESSION);
session_abort();
foreach (glob("$dir/*") as $f) unlink($f);
rmdir($dir);
$out = ob_get_clean();
echo $out;
