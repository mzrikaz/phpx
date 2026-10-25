
// ---- Date and time (natives in crates/interp/src/date/) -----------------------------

class DateError extends Error {}
class DateObjectError extends DateError {}
class DateRangeError extends DateError {}
class DateException extends Exception {}
class DateInvalidTimeZoneException extends DateException {}
class DateInvalidOperationException extends DateException {}
class DateMalformedStringException extends DateException {}
class DateMalformedIntervalStringException extends DateException {}
class DateMalformedPeriodStringException extends DateException {}

const DATE_ATOM = "Y-m-d\\TH:i:sP";
const DATE_COOKIE = "l, d-M-Y H:i:s T";
const DATE_ISO8601 = "Y-m-d\\TH:i:sO";
const DATE_ISO8601_EXPANDED = "X-m-d\\TH:i:sP";
const DATE_RFC822 = "D, d M y H:i:s O";
const DATE_RFC850 = "l, d-M-y H:i:s T";
const DATE_RFC1036 = "D, d M y H:i:s O";
const DATE_RFC1123 = "D, d M Y H:i:s O";
const DATE_RFC7231 = "D, d M Y H:i:s \\G\\M\\T";
const DATE_RFC2822 = "D, d M Y H:i:s O";
const DATE_RFC3339 = "Y-m-d\\TH:i:sP";
const DATE_RFC3339_EXTENDED = "Y-m-d\\TH:i:s.vP";
const DATE_RSS = "D, d M Y H:i:s O";
const DATE_W3C = "Y-m-d\\TH:i:sP";
const SUNFUNCS_RET_TIMESTAMP = 0;
const SUNFUNCS_RET_STRING = 1;
const SUNFUNCS_RET_DOUBLE = 2;

interface DateTimeInterface
{
    const ATOM = "Y-m-d\\TH:i:sP";
    const COOKIE = "l, d-M-Y H:i:s T";
    const ISO8601 = "Y-m-d\\TH:i:sO";
    const ISO8601_EXPANDED = "X-m-d\\TH:i:sP";
    const RFC822 = "D, d M y H:i:s O";
    const RFC850 = "l, d-M-y H:i:s T";
    const RFC1036 = "D, d M y H:i:s O";
    const RFC1123 = "D, d M Y H:i:s O";
    const RFC7231 = "D, d M Y H:i:s \\G\\M\\T";
    const RFC2822 = "D, d M Y H:i:s O";
    const RFC3339 = "Y-m-d\\TH:i:sP";
    const RFC3339_EXTENDED = "Y-m-d\\TH:i:s.vP";
    const RSS = "D, d M Y H:i:s O";
    const W3C = "Y-m-d\\TH:i:sP";
}

class DateTime implements DateTimeInterface {}

class DateTimeImmutable implements DateTimeInterface {}

class DateTimeZone
{
    const AFRICA = 1;
    const AMERICA = 2;
    const ANTARCTICA = 4;
    const ARCTIC = 8;
    const ASIA = 16;
    const ATLANTIC = 32;
    const AUSTRALIA = 64;
    const EUROPE = 128;
    const INDIAN = 256;
    const PACIFIC = 512;
    const UTC = 1024;
    const ALL = 2047;
    const ALL_WITH_BC = 4095;
    const PER_COUNTRY = 4096;
}

class DateInterval
{
    public $y = 0;
    public $m = 0;
    public $d = 0;
    public $h = 0;
    public $i = 0;
    public $s = 0;
    public $f = 0.0;
    public $invert = 0;
    public $days = false;
    public $from_string = false;

    public function __construct(string $duration)
    {
        [$this->y, $this->m, $this->d, $this->h, $this->i, $this->s, $this->f] = __phpx_interval_parse($duration);
    }

    public static function createFromDateString(string $datetime): DateInterval|false
    {
        $v = __phpx_interval_from_string($datetime);
        if ($v === false) {
            return false;
        }
        $i = new DateInterval('PT0S');
        [$i->y, $i->m, $i->d, $i->h, $i->i, $i->s, $i->f] = $v;
        return $i;
    }

    public function format(string $format): string
    {
        return __phpx_interval_format(get_object_vars($this), $format);
    }

    public static function __set_state(array $array): DateInterval
    {
        $i = new DateInterval('PT0S');
        foreach ($array as $k => $v) {
            $i->$k = $v;
        }
        return $i;
    }
}

class DatePeriod implements IteratorAggregate
{
    const EXCLUDE_START_DATE = 1;
    const INCLUDE_END_DATE = 2;

    public $start = null;
    public $current = null;
    public $end = null;
    public $interval = null;
    public $recurrences = 1;
    public $include_start_date = true;
    public $include_end_date = false;

    public function __construct($start, $interval = null, $end = null, $options = 0)
    {
        if (is_string($start)) {
            throw new DateMalformedPeriodStringException('DatePeriod::__construct(): ISO interval strings are not supported yet');
        }
        $this->start = $start;
        $this->interval = $interval;
        if (is_int($end)) {
            $this->recurrences = $end + 1;
            $this->end = null;
        } else {
            $this->end = $end;
            $this->recurrences = 1;
        }
        $this->include_start_date = !($options & self::EXCLUDE_START_DATE);
        $this->include_end_date = (bool) ($options & self::INCLUDE_END_DATE);
    }

    public function getStartDate(): DateTimeInterface { return $this->start; }
    public function getEndDate(): ?DateTimeInterface { return $this->end; }
    public function getDateInterval(): DateInterval { return $this->interval; }
    public function getRecurrences(): ?int { return $this->end === null ? $this->recurrences - 1 : null; }

    public function getIterator(): Iterator
    {
        $out = [];
        $class = get_class($this->start);
        $cur = DateTimeImmutable::createFromInterface($this->start);
        $n = 0;
        if (!$this->include_start_date) {
            $cur = $cur->add($this->interval);
        }
        while (true) {
            if ($this->end !== null) {
                if ($this->include_end_date ? $cur > $this->end : $cur >= $this->end) {
                    break;
                }
            } elseif ($n >= $this->recurrences) {
                break;
            }
            $out[] = $class === DateTimeImmutable::class ? $cur : $class::createFromInterface($cur);
            $n++;
            $cur = $cur->add($this->interval);
        }
        return new ArrayIterator($out);
    }
}

function date_create(string $datetime = 'now', ?DateTimeZone $timezone = null): DateTime|false
{
    try {
        return new DateTime($datetime, $timezone);
    } catch (Exception) {
        return false;
    }
}

function date_create_immutable(string $datetime = 'now', ?DateTimeZone $timezone = null): DateTimeImmutable|false
{
    try {
        return new DateTimeImmutable($datetime, $timezone);
    } catch (Exception) {
        return false;
    }
}

function date_create_from_format(string $format, string $datetime, ?DateTimeZone $timezone = null): DateTime|false
{
    return DateTime::createFromFormat($format, $datetime, $timezone);
}

function date_create_immutable_from_format(string $format, string $datetime, ?DateTimeZone $timezone = null): DateTimeImmutable|false
{
    return DateTimeImmutable::createFromFormat($format, $datetime, $timezone);
}

function date_format(DateTimeInterface $object, string $format): string { return $object->format($format); }
function date_modify(DateTime $object, string $modifier): DateTime|false { return $object->modify($modifier); }
function date_add(DateTime $object, DateInterval $interval): DateTime { return $object->add($interval); }
function date_sub(DateTime $object, DateInterval $interval): DateTime { return $object->sub($interval); }
function date_diff(DateTimeInterface $baseObject, DateTimeInterface $targetObject, bool $absolute = false): DateInterval { return $baseObject->diff($targetObject, $absolute); }
function date_timezone_get(DateTimeInterface $object): DateTimeZone|false { return $object->getTimezone(); }
function date_timezone_set(DateTime $object, DateTimeZone $timezone): DateTime { return $object->setTimezone($timezone); }
function date_offset_get(DateTimeInterface $object): int { return $object->getOffset(); }
function date_timestamp_get(DateTimeInterface $object): int { return $object->getTimestamp(); }
function date_timestamp_set(DateTime $object, int $timestamp): DateTime { return $object->setTimestamp($timestamp); }
function date_date_set(DateTime $object, int $year, int $month, int $day): DateTime { return $object->setDate($year, $month, $day); }
function date_isodate_set(DateTime $object, int $year, int $week, int $dayOfWeek = 1): DateTime { return $object->setISODate($year, $week, $dayOfWeek); }
function date_time_set(DateTime $object, int $hour, int $minute, int $second = 0, int $microsecond = 0): DateTime { return $object->setTime($hour, $minute, $second, $microsecond); }
function date_get_last_errors(): array|false { return false; }
function date_interval_format(DateInterval $object, string $format): string { return $object->format($format); }
function date_interval_create_from_date_string(string $datetime): DateInterval|false { return DateInterval::createFromDateString($datetime); }

function timezone_open(string $timezone): DateTimeZone|false
{
    try {
        return new DateTimeZone($timezone);
    } catch (Exception) {
        trigger_error('timezone_open(): Unknown or bad timezone (' . $timezone . ')', E_USER_WARNING);
        return false;
    }
}

function timezone_name_get(DateTimeZone $object): string { return $object->getName(); }
function timezone_offset_get(DateTimeZone $object, DateTimeInterface $datetime): int { return $object->getOffset($datetime); }
function timezone_transitions_get(DateTimeZone $object, int $timestampBegin = PHP_INT_MIN, int $timestampEnd = PHP_INT_MAX): array|false { return $object->getTransitions($timestampBegin, $timestampEnd); }
function timezone_location_get(DateTimeZone $object): array|false { return $object->getLocation(); }
function timezone_abbreviations_list(): array { return DateTimeZone::listAbbreviations(); }
