<?php
$base = sys_get_temp_dir() . "/phpx_splfs_" . getmypid(); @mkdir("$base/sub/deep", 0777, true); file_put_contents("$base/a.txt", "a\n"); file_put_contents("$base/sub/b.md", "bb\n"); file_put_contents("$base/sub/deep/c.txt", "ccc\n"); file_put_contents("$base/data.csv", "x,y\n1,2\n\n3,4\n"); file_put_contents("$base/ab.dat", "a\nb\n");
$f = new SplFileInfo("$base/sub/b.md/");
var_dump($f->getFilename(), $f->getExtension(), $f->getBasename('.md'), $f->getPath() === "$base/sub", $f->getSize(), $f->isFile(), $f->isDir(), $f->getType(), (string) $f === "$base/sub/b.md");
$names = []; foreach (new DirectoryIterator($base) as $k => $d) { $names[] = ($d->isDot() ? 'dot:' : '') . $d->getFilename() . ($d->isDir() ? '/' : ''); } sort($names); echo implode(' ', $names), "\n";
$names = []; foreach (new FilesystemIterator($base) as $k => $d) { $names[] = basename($k) . '=' . get_class($d) . ':' . $d->getFilename(); } sort($names); echo implode(' ', $names), "\n";
$all = []; $it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($base, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST);
foreach ($it as $p => $info) { $all[] = $it->getSubPathname() . ($info->isDir() ? '/' : '') . '@' . $it->getDepth(); } sort($all); echo implode(' ', $all), "\n";
$c = []; foreach (new GlobIterator("$base/*.txt") as $g) $c[] = $g->getFilename(); echo implode(',', $c), "\n";
$o = new SplFileObject("$base/data.csv"); $o->setFlags(SplFileObject::READ_CSV | SplFileObject::SKIP_EMPTY | SplFileObject::READ_AHEAD | SplFileObject::DROP_NEW_LINE);
foreach ($o as $n => $row) echo $n, ':', json_encode($row), ' '; echo "\n";
$o = new SplFileObject("$base/data.csv"); foreach ($o as $n => $l) echo $n, '=', json_encode($l), ' '; echo "\n";
$t = new SplTempFileObject(); $t->fwrite("hi\nthere\n"); $t->rewind(); foreach ($t as $l) echo trim($l), '|'; echo "\n";
try { new DirectoryIterator("$base/nope"); } catch (UnexpectedValueException $e) { echo get_class($e), ': ', str_replace($base, 'B', $e->getMessage()), "\n"; }
try { (new SplFileInfo("$base/nope"))->getSize(); } catch (RuntimeException $e) { echo str_replace($base, 'B', $e->getMessage()), "\n"; }
$ri = new RegexIterator(new ArrayIterator(['apple', 'banana', 'cherry']), '/an/'); echo implode(',', iterator_to_array($ri)), "\n";
$m = new MultipleIterator(); $m->attachIterator(new ArrayIterator([1, 2])); $m->attachIterator(new ArrayIterator(['a', 'b'])); foreach ($m as $v) echo json_encode($v); echo "\n";
$h = fopen("$base/ab.dat", 'r');
var_dump(fgets($h), feof($h), fgets($h), feof($h), fgets($h), feof($h));
$o = new SplFileObject("$base/ab.dat"); foreach ($o as $k => $l) echo $k, '=', json_encode($l), ' '; echo "\n";
$o = new SplFileObject("$base/ab.dat"); $o->setFlags(SplFileObject::DROP_NEW_LINE); foreach ($o as $k => $l) echo $k, '=', json_encode($l), ' '; echo "\n";
$o = new SplFileObject("$base/ab.dat"); $o->setFlags(SplFileObject::READ_AHEAD); foreach ($o as $k => $l) echo $k, '=', json_encode($l), ' '; echo "\n";
$o = new SplFileObject("$base/ab.dat"); $o->setFlags(SplFileObject::SKIP_EMPTY); foreach ($o as $k => $l) echo $k, '=', json_encode($l), ' '; echo "\n";
$o = new SplFileObject("$base/ab.dat"); $o->setFlags(SplFileObject::SKIP_EMPTY|SplFileObject::DROP_NEW_LINE); foreach ($o as $k => $l) echo $k, '=', json_encode($l), ' '; echo "\n";
foreach (["sub/deep/c.txt", "sub/b.md", "a.txt", "data.csv", "ab.dat"] as $f) unlink("$base/$f"); rmdir("$base/sub/deep"); rmdir("$base/sub"); rmdir($base);
