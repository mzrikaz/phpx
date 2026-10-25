<?php
[$a, $b] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, STREAM_IPPROTO_IP);
var_dump(get_resource_type($a), fwrite($a, "ping\n"), fgets($b));
fwrite($b, "pong");
fclose($b);
var_dump(stream_get_contents($a), feof($a));
fclose($a);
[$x, $y] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, STREAM_IPPROTO_IP);
var_dump(stream_socket_sendto($x, "hello"), stream_socket_recvfrom($y, 5));
var_dump(@get_headers('ftp://example.invalid/'));
