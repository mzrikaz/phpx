<?php
var_dump(getprotobyname('tcp'), getprotobyname('udp'), getprotobynumber(6), getprotobyname('nope-proto'));
var_dump(gethostbyaddr('127.0.0.1') !== false, @gethostbyaddr('not-an-ip'));
var_dump(ftok(__FILE__, 'a') !== 0);
try { ftok(__FILE__, 'ab'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$i = net_get_interfaces();
var_dump(is_array($i), isset($i['lo']['unicast']));
