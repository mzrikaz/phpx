
// ---- ext/redis (natives in crates/interp/src/redis/) -----------------------------------

class RedisException extends RuntimeException
{
}

class Redis
{
    public const REDIS_NOT_FOUND = 0;
    public const REDIS_STRING = 1;
    public const REDIS_SET = 2;
    public const REDIS_LIST = 3;
    public const REDIS_ZSET = 4;
    public const REDIS_HASH = 5;
    public const REDIS_STREAM = 6;
    public const REDIS_VECTORSET = 7;
    public const ATOMIC = 0;
    public const MULTI = 1;
    public const PIPELINE = 2;
    public const OPT_SERIALIZER = 1;
    public const OPT_PREFIX = 2;
    public const OPT_READ_TIMEOUT = 3;
    public const OPT_TCP_KEEPALIVE = 6;
    public const OPT_COMPRESSION = 7;
    public const OPT_REPLY_LITERAL = 8;
    public const OPT_COMPRESSION_LEVEL = 9;
    public const OPT_NULL_MULTIBULK_AS_NULL = 10;
    public const OPT_PACK_IGNORE_NUMBERS = 15;
    public const SERIALIZER_NONE = 0;
    public const SERIALIZER_PHP = 1;
    public const SERIALIZER_JSON = 4;
    public const COMPRESSION_NONE = 0;
    public const OPT_SCAN = 4;
    public const SCAN_RETRY = 1;
    public const SCAN_NORETRY = 0;
    public const SCAN_PREFIX = 2;
    public const SCAN_NOPREFIX = 3;
    public const BEFORE = 'before';
    public const AFTER = 'after';
    public const LEFT = 'left';
    public const RIGHT = 'right';
    public const OPT_MAX_RETRIES = 11;
    public const OPT_BACKOFF_ALGORITHM = 12;
    public const BACKOFF_ALGORITHM_DEFAULT = 0;
    public const BACKOFF_ALGORITHM_CONSTANT = 6;
    public const BACKOFF_ALGORITHM_UNIFORM = 5;
    public const BACKOFF_ALGORITHM_EXPONENTIAL = 4;
    public const BACKOFF_ALGORITHM_FULL_JITTER = 2;
    public const BACKOFF_ALGORITHM_EQUAL_JITTER = 3;
    public const BACKOFF_ALGORITHM_DECORRELATED_JITTER = 1;
    public const OPT_BACKOFF_BASE = 13;
    public const OPT_BACKOFF_CAP = 14;

    public function scan(&$iterator, $pattern = null, $count = 0, $type = null)
    {
        [$iterator, $r] = $this->__phpx_scan('SCAN', null, $iterator, $pattern, $count);
        return $r;
    }

    public function hScan($key, &$iterator, $pattern = null, $count = 0)
    {
        [$iterator, $r] = $this->__phpx_scan('HSCAN', $key, $iterator, $pattern, $count);
        return $r;
    }

    public function sScan($key, &$iterator, $pattern = null, $count = 0)
    {
        [$iterator, $r] = $this->__phpx_scan('SSCAN', $key, $iterator, $pattern, $count);
        return $r;
    }

    public function zScan($key, &$iterator, $pattern = null, $count = 0)
    {
        [$iterator, $r] = $this->__phpx_scan('ZSCAN', $key, $iterator, $pattern, $count);
        return $r;
    }
}
