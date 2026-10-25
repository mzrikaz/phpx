<?php
$x = 1; $a = ["r" => &$x, "v" => 2, 0 => &$x];
$r1 = ReflectionReference::fromArrayElement($a, "r"); $r2 = ReflectionReference::fromArrayElement($a, 0);
var_dump(ReflectionReference::fromArrayElement($a, "v"), $r1 instanceof ReflectionReference, $r1->getId() === $r2->getId(), strlen($r1->getId()));
try { ReflectionReference::fromArrayElement($a, "zz"); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
