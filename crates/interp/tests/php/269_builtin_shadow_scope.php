<?php
// A namespaced function named like a built-in affects only calls in its own namespace.
namespace Other { function strlen($s) { return "other strlen"; } function is_null($x) { return "other is_null"; } }
namespace App {
    function probe($v) { return [is_null($v), \is_null($v), strlen("abc"), count([1, 2]), is_array($v), sqrt(16.0)]; }
}
namespace Other {
    function probe($v) { return [is_null($v), \is_null($v), strlen("abc")]; }
}
namespace Late {
    function probe() { return [strlen("abcd"), is_string("x")]; }
}
namespace {
    for ($i = 0; $i < 3; $i++) {
        echo json_encode([App\probe(null), Other\probe(null), Late\probe()]), "\n";
        if ($i == 1) { eval('namespace Late; function strlen($s) { return -1; }'); }
    }
    use function Other\strlen as olen;
    echo olen("x"), "\n";
}
