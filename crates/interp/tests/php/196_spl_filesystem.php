<?php
$base = sys_get_temp_dir() . '/phpx_spl_' . getmypid();
@mkdir($base); @mkdir("$base/sub"); @mkdir("$base/sub/deep");
file_put_contents("$base/a.txt", "x"); file_put_contents("$base/b.php", "y"); file_put_contents("$base/sub/c.TXT", "z"); file_put_contents("$base/sub/deep/d", "w");
@symlink("$base/sub", "$base/link");
function norm($s) { global $base; return str_replace($base, 'BASE', (string)$s); }
function names($it) { $o = []; foreach ($it as $k => $v) { $o[] = norm($k) . '=>' . norm(is_object($v) ? get_class($v) . ':' . $v->getPathname() : $v); } sort($o); return $o; }
print_r(names(new FilesystemIterator($base)));
print_r(names(new FilesystemIterator($base, FilesystemIterator::KEY_AS_FILENAME | FilesystemIterator::CURRENT_AS_PATHNAME)));
$d = []; foreach (new DirectoryIterator($base) as $k => $f) { $d[] = [$f->getFilename(), $f->isDot(), $f->isDir(), $f->isFile(), $f->isLink(), $f->getExtension(), norm($f->getPath()), norm($f->getPathname())]; } sort($d); echo json_encode($d), "\n";
$r = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($base, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST);
$o = []; foreach ($r as $p => $f) { $o[] = norm($p) . '|' . $r->getSubPathname() . '|' . ($f->isDir() ? 'D' : 'F') . '|' . norm($f->getRealPath()) . '|' . $f->getBasename('.txt'); } sort($o); print_r($o);
$r2 = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($base, FilesystemIterator::SKIP_DOTS | FilesystemIterator::FOLLOW_SYMLINKS));
$o = []; foreach ($r2 as $p => $f) $o[] = norm($p); sort($o); print_r($o);
$g = new GlobIterator("$base/*.txt"); echo count($g), "\n"; foreach ($g as $k => $f) echo norm($k), ' ', $f->getFilename(), "\n";
$i = new SplFileInfo("$base/sub/"); var_dump($i->getFilename(), norm($i->getPath()), norm($i->getPathname()), $i->isDir(), $i->getExtension(), norm($i->getRealPath()), norm((string)$i));
$i = new SplFileInfo("nofile.tar.gz"); var_dump($i->getFilename(), $i->getPath(), $i->getExtension(), $i->getBasename('.gz'), $i->isFile(), $i->getRealPath());
$di = new DirectoryIterator($base); $di->seek(1); try { $di->seek(99); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }
try { new DirectoryIterator("$base/nope"); } catch (UnexpectedValueException $e) { echo norm($e->getMessage()), "\n"; }
class MyInfo extends SplFileInfo {}
$f = new FilesystemIterator($base); $f->setInfoClass('MyInfo'); foreach ($f as $x) { echo get_class($x), "\n"; break; }
var_dump(new SplFileInfo('/tmp/x/y.txt'));
unlink("$base/link"); unlink("$base/sub/deep/d"); rmdir("$base/sub/deep"); unlink("$base/sub/c.TXT"); rmdir("$base/sub"); unlink("$base/a.txt"); unlink("$base/b.php"); rmdir($base);
