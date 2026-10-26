<?php
foreach (['memory_limit', 'precision', 'session.name', 'date.timezone', 'mbstring.language', 'pcre.jit',
          'zend.assertions', 'serialize_precision', 'default_charset', 'max_input_vars', 'session.sid_length',
          'arg_separator.output', 'highlight.string', 'no.such.directive', 'disable_classes'] as $k) {
    echo $k, ' = ', var_export(ini_get($k), true), "\n";
}
var_dump(ini_set('memory_limit', '256M'), ini_get('memory_limit'));
var_dump(ini_set('allow_url_fopen', '0'), ini_get('allow_url_fopen'));
var_dump(ini_set('no.such.directive', '1'));
$all = ini_get_all('session');
echo count($all), ' ', json_encode($all['session.name']), "\n";
echo json_encode(ini_get_all('date', false)), "\n";
var_dump(@ini_get_all('nope'));
echo json_encode(array_slice(ini_get_all(null, false), 0, 3)), "\n";
ob_start(); phpinfo(INFO_MODULES); $s = ob_get_clean();
foreach (['json', 'ctype', 'date', 'session'] as $m) {
    preg_match('/\n' . $m . '\n\n(.*?)\n\n\S[^\n=]*\n\n/s', $s, $mm);
    echo "[$m]\n", $mm[1] ?? 'missing', "\n";
}
