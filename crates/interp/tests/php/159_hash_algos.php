<?php
$inputs = ['', 'a', 'abc', 'The quick brown fox jumps over the lazy dog', str_repeat('xyz0123456789', 37)];
foreach (["md2","md4","md5","sha1","sha224","sha256","sha384","sha512/224","sha512/256","sha512","sha3-224","sha3-256","sha3-384","sha3-512","ripemd128","ripemd160","ripemd256","ripemd320","whirlpool","tiger128,3","tiger160,3","tiger192,3","gost","gost-crypto","adler32","crc32","crc32b","crc32c","fnv132","fnv1a32","fnv164","fnv1a64","joaat","murmur3a","murmur3c","murmur3f","xxh32","xxh64","xxh3","xxh128"] as $algo) {
    $line = [];
    foreach ($inputs as $in) $line[] = hash($algo, $in);
    echo str_pad($algo, 12), implode(' ', $line), "\n";
}
foreach (['murmur3a', 'murmur3c', 'murmur3f', 'xxh32', 'xxh64', 'xxh3', 'xxh128'] as $a) echo "$a seed ", hash($a, 'data', false, ['seed' => 42]), "\n";
echo hash('xxh3', 'data', false, ['secret' => str_repeat('s', 136)]), "\n";
echo hash_hmac('sha3-256', 'data', 'key'), " ", hash_hmac('ripemd160', 'data', 'key'), "\n";
$c = hash_init('xxh128'); hash_update($c, 'ab'); hash_update($c, 'c'); echo hash_final($c), "\n";
try { hash_hmac('crc32b', 'x', 'k'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
