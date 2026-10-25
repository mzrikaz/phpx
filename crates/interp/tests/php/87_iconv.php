<?php
var_dump(bin2hex(iconv('UTF-8', 'ISO-8859-1', "caf\u{e9}")), iconv('ISO-8859-1', 'UTF-8', "caf\xe9"));
var_dump(bin2hex(iconv('UTF-8', 'UTF-16BE', "A\u{20ac}")));
var_dump(@iconv('UTF-8', 'ISO-8859-1', "\xff\xfe"));
var_dump(iconv_strlen("h\u{e9}llo"), iconv_strlen("caf\xe9", 'ISO-8859-1'));
var_dump(iconv_substr("h\u{e9}llo w\u{f6}rld", 1, 4), iconv_substr("h\u{e9}llo", -3), iconv_substr("abc", 5), iconv_substr("h\u{e9}llo", 1, -1));
var_dump(iconv_strpos("h\u{e9}llo h\u{e9}llo", "llo"), iconv_strpos("h\u{e9}llo h\u{e9}llo", "llo", 4), iconv_strrpos("h\u{e9}llo h\u{e9}llo", "llo"), iconv_strpos("abc", "x"));
var_dump(iconv_mime_decode("Subject: =?UTF-8?B?UHLDvGZ1bmcgUHLDvGZ1bmc=?="));
var_dump(iconv_mime_decode("=?ISO-8859-1?Q?Andr=E9?= Pirard <PIRARD@vm1.ulg.ac.be>"));
var_dump(iconv_mime_decode("=?UTF-8?Q?a?= =?UTF-8?Q?b?= plain =?UTF-8?B?Yw==?="));
$headers = "Subject: =?UTF-8?B?UHLDvGZ1bmc=?=\r\nTo: example@example.com\r\nX-A: one\r\nX-A: =?UTF-8?Q?tw=C3=B6?=\r\n";
var_dump(iconv_mime_decode_headers($headers));
var_dump(iconv_mime_encode("Subject", "Prüfung Prüfung", ['scheme' => 'B', 'input-charset' => 'UTF-8', 'output-charset' => 'UTF-8']));
var_dump(iconv_mime_encode("Subject", "plain", ['scheme' => 'Q']));
var_dump(iconv_get_encoding('internal_encoding'), iconv_get_encoding());
