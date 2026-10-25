<?php
var_dump(mb_split('\s*,\s*', 'a , b,c'), mb_split('', 'abc'), mb_strripos('ÄbcäBC', 'äb'), mb_strrchr('a/b/c', '/'), mb_strrchr('a/b/c', '/', true), mb_stristr('HÉLLO wörld', 'éll'), mb_strrichr('aXbxC', 'x'), mb_strcut('añb', 1, 2), mb_strcut('añb', 2, 2));
var_dump(mb_ereg('(\d+)-(\d+)', 'x 12-34 y', $r), $r, mb_eregi('ABC', 'xabcx'), mb_ereg_replace('[aeiou]', '_', 'educación'), mb_ereg_match('a', 'abc'), mb_ereg_match('b', 'abc'));
var_dump(mb_encode_numericentity('aé€', [0x80, 0x10ffff, 0, 0x1fffff], 'UTF-8'), mb_decode_numericentity('a&#233;&#8364;', [0x80, 0x10ffff, 0, 0x1fffff], 'UTF-8'));
var_dump(mb_decode_mimeheader('=?UTF-8?B?w6l0w6k=?= =?UTF-8?Q?caf=C3=A9_x?='), mb_language(), mb_preferred_mime_name('utf8'), mb_substitute_character(), mb_detect_order());
var_dump(mb_ereg_replace_callback('\d', fn($m) => $m[0] * 2, 'a1b2'));
