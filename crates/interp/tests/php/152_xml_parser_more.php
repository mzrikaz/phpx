<?php
class H {
    public $out = [];
    function s($p, $n, $a) { $this->out[] = "S:$n:" . json_encode($a); }
    function e($p, $n) { $this->out[] = "E:$n"; }
    function c($p, $d) { $this->out[] = "C:" . json_encode($d); }
}
$h = new H;
$p = xml_parser_create();
xml_set_object($p, $h);
xml_set_element_handler($p, 's', 'e');
xml_set_character_data_handler($p, 'c');
xml_parser_set_option($p, XML_OPTION_CASE_FOLDING, false);
foreach (["<doc><it", "em a='1'>he", "llo</item><item>x", "</item></doc>"] as $i => $chunk) {
    var_dump(xml_parse($p, $chunk, $i === 3));
    echo implode(" ", $h->out), "\n"; $h->out = [];
}
$p = xml_parser_create_ns('UTF-8', '#');
xml_set_element_handler($p, function ($p, $n, $a) { echo "S $n ", json_encode($a), "\n"; }, function ($p, $n) { echo "E $n\n"; });
xml_set_start_namespace_decl_handler($p, function ($p, $pre, $uri) { echo "NS ", var_export($pre, true), " $uri\n"; });
xml_parse($p, '<r xmlns="urn:d" xmlns:x="urn:x"><x:a x:k="1" k="2"/><b/></r>', true);
$p = xml_parser_create(); xml_parser_set_option($p, XML_OPTION_TARGET_ENCODING, 'ISO-8859-1');
xml_set_character_data_handler($p, function ($p, $d) { echo bin2hex($d), "\n"; });
xml_parse($p, "<a>\xc3\xa9\xe2\x82\xac</a>", true);
foreach (['<a></b>', '<a>', '<a attr></a>', '', 'text', '<a>&nope;</a>', '<a><!-- x --></a>extra'] as $bad) {
    $p = xml_parser_create();
    $r = xml_parse($p, $bad, true);
    echo json_encode($bad), " => $r ", xml_get_error_code($p), " ", xml_error_string(xml_get_error_code($p)), " line ", xml_get_current_line_number($p), "\n";
}
$p = xml_parser_create(); xml_parse_into_struct($p, '<root><a>1</a><a>2</a><b x="y">t<c/>u</b></root>', $v, $i); echo json_encode($v), "\n", json_encode($i), "\n";
var_dump(xml_parser_get_option($p, XML_OPTION_CASE_FOLDING), xml_parser_get_option($p, XML_OPTION_TARGET_ENCODING));
