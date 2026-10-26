<?php
class U { public ?U $next = null; public $arr = null; public function name() { return "u"; } }
function t($u) { return [$u->next?->name(), $u?->name(), $u->next?->next->name(), $u->next?->next?->arr['x'], $u->next?->arr[0]->z, isset($u->next?->next)]; }
for ($i = 0; $i < 3; $i++) var_dump(t(new U));
