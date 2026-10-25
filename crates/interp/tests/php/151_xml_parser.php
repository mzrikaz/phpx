<?php
$p = xml_parser_create();
xml_set_element_handler($p, function ($p, $n, $a) { echo "start $n ", json_encode($a), "\n"; }, function ($p, $n) { echo "end $n\n"; });
xml_set_character_data_handler($p, function ($p, $d) { echo "cdata ", json_encode($d), "\n"; });
xml_set_processing_instruction_handler($p, function ($p, $t, $d) { echo "pi $t ", json_encode($d), "\n"; });
xml_set_default_handler($p, function ($p, $d) { echo "default ", json_encode($d), "\n"; });
var_dump(xml_parse($p, "<?xml version=\"1.0\"?>\n<!-- c --><root a=\"1\" b='x &amp; y'>line1\nline2 &lt;tag&gt; &#65; <![CDATA[ cd <x> ]]><child/>t\xc3\xa9xt<?pi data?></root>", true));
var_dump(xml_get_error_code($p));
$p = xml_parser_create(); xml_parser_set_option($p, XML_OPTION_CASE_FOLDING, 0); xml_parser_set_option($p, XML_OPTION_SKIP_WHITE, 1);
xml_parse_into_struct($p, "<a x=\"1\">\n  <b>hi</b>\n  <c/>\n  text<d>x</d>more\n</a>", $vals, $idx);
print_r($vals); print_r($idx);
$p = xml_parser_create(); var_dump(xml_parse($p, "<a><b></a>", true), xml_get_error_code($p), xml_error_string(xml_get_error_code($p)), xml_get_current_line_number($p));
$p = xml_parser_create(); var_dump(xml_parse($p, "<a>", false), xml_parse($p, "x</a>", true));
