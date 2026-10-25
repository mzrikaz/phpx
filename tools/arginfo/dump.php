<?php
// Dump internal function signatures as JSON lines: name, params, return type.
function t($t) { return $t === null ? null : (string)$t; }
$out = [];
foreach (get_defined_functions()['internal'] as $fn) {
    $r = new ReflectionFunction($fn);
    $ps = [];
    foreach ($r->getParameters() as $p) {
        $ps[] = [
            'name' => $p->getName(),
            'type' => t($p->getType()),
            'opt' => $p->isOptional(),
            'ref' => $p->isPassedByReference(),
            'var' => $p->isVariadic(),
            'def' => $p->isDefaultValueAvailable() ? $p->getDefaultValueConstantName() ?? var_export($p->getDefaultValue(), true) : null,
        ];
    }
    $out[] = ['name' => $r->getName(), 'ext' => $r->getExtensionName(), 'ret' => t($r->getReturnType()), 'params' => $ps, 'deprecated' => $r->isDeprecated()];
}
echo json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES), "\n";
