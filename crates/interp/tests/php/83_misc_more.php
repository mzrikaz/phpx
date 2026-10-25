<?php
var_dump(password_algos());
foreach (["1K", "2m", "1G", "0x10", "010", "0o10", "0b11", " 5k ", "1.5M", "-1", "abc", "5x", "", "1KB"] as $q) { echo json_encode($q), " => "; var_dump(@ini_parse_quantity($q)); }
var_dump(image_type_to_mime_type(IMAGETYPE_PNG), image_type_to_mime_type(IMAGETYPE_JPEG), image_type_to_mime_type(IMAGETYPE_GIF), image_type_to_mime_type(IMAGETYPE_WEBP), image_type_to_mime_type(IMAGETYPE_ICO), image_type_to_mime_type(999));
var_dump(image_type_to_extension(IMAGETYPE_PNG), image_type_to_extension(IMAGETYPE_JPEG, false), image_type_to_extension(IMAGETYPE_TIFF_II), image_type_to_extension(999));
$png = base64_decode("iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP8z8DwnwEIGBkZGQBFEgYFd3gl2QAAAABJRU5ErkJggg==");
var_dump(getimagesizefromstring($png));
$gif = "GIF89a\x05\x00\x07\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x05\x00\x07\x00\x00\x02\x02D\x01\x00;";
var_dump(getimagesizefromstring($gif));
$jpg = "\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xc0\x00\x11\x08\x00\x20\x00\x40\x03\x01\x22\x00\x02\x11\x01\x03\x11\x01\xff\xd9";
var_dump(getimagesizefromstring($jpg));
$bmp = "BM" . pack("V", 70) . "\0\0\0\0" . pack("V", 54) . pack("V", 40) . pack("V", 9) . pack("V", 4) . pack("v", 1) . pack("v", 24) . str_repeat("\0", 24);
var_dump(getimagesizefromstring($bmp));
var_dump(@getimagesizefromstring("notanimage"));
highlight_string('<?php
// comment
$a = "str" . 42 . FOO; /* block */
echo strlen($a) . __LINE__; ?>
<b>html & stuff</b>
<?php if ($x > 1) { return <<<EOT
  heredoc $a
  EOT; }
');
echo "\n";
var_dump(highlight_string('<?= "x\ty" ?>', true));
$f = sys_get_temp_dir() . '/phpx_strip_' . getmypid() . '.php';
file_put_contents($f, "<?php\n/** doc */\nfunction  f( \$a ,\n  \$b ) {   // c\n  return \$a+\$b;\n}\n\$x = <<<EOT\n  text\n  EOT;\necho f(1, 2);\n");
var_dump(php_strip_whitespace($f));
file_put_contents($f, '<html><head><meta name="Author" content="Jane Doe"><meta name="keywords" content="php documentation"><META NAME="geo.position" CONTENT="49.33;-86.59"><meta name="description" content=\'a "quoted" thing\'></head><body><meta name="ignored" content="x"></body></html>');
var_dump(get_meta_tags($f));
file_put_contents($f, "12 apples\nsecond line\n");
$h = fopen($f, 'r');
var_dump(fscanf($h, "%d %s"));
$n = fscanf($h, "%s %s", $w1, $w2);
var_dump($n, $w1, $w2, fsync($h), fdatasync($h));
fclose($h);
unlink($f);
$d = sys_get_temp_dir() . '/phpx_dir_' . getmypid();
mkdir($d);
touch("$d/one");
$dir = dir($d);
var_dump(get_class($dir), $dir->path === $d, is_resource($dir->handle));
$names = [];
while (($e = $dir->read()) !== false) { $names[] = $e; }
sort($names);
var_dump($names);
$dir->rewind();
var_dump($dir->read() !== false);
$dir->close();
unlink("$d/one");
rmdir($d);
var_dump(iptcparse("\x1c\x02\x05\x00\x05Title\x1c\x02\x19\x00\x03one\x1c\x02\x19\x00\x03two"), iptcparse("nothing"));
var_dump(getopt("ab:c::", ["long", "req:"]));
var_dump(register_tick_function('strlen'), realpath_cache_get());
var_dump(@get_browser());
