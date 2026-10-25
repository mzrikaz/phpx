
// ---- SPL: files and directories ----------------------------------------------------

class SplFileInfo implements Stringable
{
    private string $pathName = '';
    private string $fileName = '';
    private string $__phpx_fileClass = SplFileObject::class;
    private string $__phpx_infoClass = SplFileInfo::class;

    public function __construct(string $filename)
    {
        $this->__phpx_setPath($filename);
    }

    /** @internal */
    protected function __phpx_setPath(string $path): void
    {
        $len = strlen($path);
        while ($len > 1 && $path[$len - 1] === '/') {
            $len--;
        }
        $path = substr($path, 0, $len);
        $this->pathName = $path;
        $i = strrpos($path, '/');
        $this->fileName = $i === false ? $path : substr($path, $i + 1);
    }

    public function getPath(): string
    {
        $i = strrpos($this->getPathname(), '/');
        return $i === false ? '' : substr($this->getPathname(), 0, $i);
    }

    public function getFilename(): string { return $this->fileName; }

    public function getExtension(): string
    {
        $f = $this->getFilename();
        $i = strrpos($f, '.');
        return $i === false ? '' : substr($f, $i + 1);
    }

    public function getBasename(string $suffix = ''): string { return basename($this->getFilename(), $suffix); }

    public function getPathname(): string { return $this->pathName; }

    /** @internal */
    private function __phpx_stat(string $what, mixed $v): mixed
    {
        if ($v === false) {
            throw new RuntimeException(sprintf('SplFileInfo::%s(): stat failed for %s', $what, $this->getPathname()));
        }
        return $v;
    }

    public function getPerms(): int|false { return $this->__phpx_stat('getPerms', @fileperms($this->getPathname())); }
    public function getInode(): int|false { return $this->__phpx_stat('getInode', @fileinode($this->getPathname())); }
    public function getSize(): int|false { return $this->__phpx_stat('getSize', @filesize($this->getPathname())); }
    public function getOwner(): int|false { return $this->__phpx_stat('getOwner', @fileowner($this->getPathname())); }
    public function getGroup(): int|false { return $this->__phpx_stat('getGroup', @filegroup($this->getPathname())); }
    public function getATime(): int|false { return $this->__phpx_stat('getATime', @fileatime($this->getPathname())); }
    public function getMTime(): int|false { return $this->__phpx_stat('getMTime', @filemtime($this->getPathname())); }
    public function getCTime(): int|false { return $this->__phpx_stat('getCTime', @filectime($this->getPathname())); }
    public function getType(): string|false { return $this->__phpx_stat('getType', @filetype($this->getPathname())); }
    public function isWritable(): bool { return is_writable($this->getPathname()); }
    public function isReadable(): bool { return is_readable($this->getPathname()); }
    public function isExecutable(): bool { return is_executable($this->getPathname()); }
    public function isFile(): bool { return is_file($this->getPathname()); }
    public function isDir(): bool { return is_dir($this->getPathname()); }
    public function isLink(): bool { return is_link($this->getPathname()); }

    public function getLinkTarget(): string|false
    {
        $t = @readlink($this->getPathname());
        if ($t === false) {
            throw new RuntimeException('Unable to read link ' . $this->getPathname() . ', error: Invalid argument');
        }
        return $t;
    }

    public function getRealPath(): string|false { return realpath($this->getPathname()); }

    public function getFileInfo(?string $class = null): SplFileInfo
    {
        $c = $class ?? $this->__phpx_infoClass;
        return new $c($this->getPathname());
    }

    public function getPathInfo(?string $class = null): ?SplFileInfo
    {
        $c = $class ?? $this->__phpx_infoClass;
        $p = $this->getPath();
        return $p === '' ? null : new $c($p);
    }

    public function openFile(string $mode = 'r', bool $useIncludePath = false, $context = null): SplFileObject
    {
        $c = $this->__phpx_fileClass;
        return new $c($this->getPathname(), $mode, $useIncludePath, $context);
    }

    public function setFileClass(string $class = SplFileObject::class): void { $this->__phpx_fileClass = $class; }
    public function setInfoClass(string $class = SplFileInfo::class): void { $this->__phpx_infoClass = $class; }
    public function __toString(): string { return $this->getPathname(); }
}

class php_user_filter
{
    public string $filtername = "";
    public mixed $params = "";
    public $stream = null;

    public function filter($in, $out, &$consumed, bool $closing): int
    {
        return PSFS_ERR_FATAL;
    }

    public function onCreate(): bool
    {
        return true;
    }

    public function onClose(): void
    {
    }
}

final class StreamBucket
{
    public $bucket;
    public string $data;
    public int $datalen;
}

final class Directory
{
    public readonly string $path;
    public readonly mixed $handle;

    public function read(): string|false
    {
        return readdir($this->handle);
    }

    public function rewind(): void
    {
        rewinddir($this->handle);
    }

    public function close(): void
    {
        closedir($this->handle);
    }
}

class DirectoryIterator extends SplFileInfo implements SeekableIterator
{
    /** @internal */
    protected string $__phpx_dir = '';
    /** @internal */
    protected array $__phpx_entries = [];
    /** @internal */
    protected int $__phpx_pos = 0;

    public function __construct(string $directory)
    {
        if ($directory === '') {
            throw new ValueError(static::class . '::__construct(): Argument #1 ($directory) cannot be empty');
        }
        $this->__phpx_open($directory);
    }

    /** @internal */
    protected function __phpx_open(string $directory): void
    {
        $len = strlen($directory);
        while ($len > 1 && $directory[$len - 1] === '/') {
            $len--;
        }
        $this->__phpx_dir = substr($directory, 0, $len);
        $h = @opendir($directory);
        if ($h === false) {
            throw new UnexpectedValueException(static::class . '::__construct(' . $directory . '): Failed to open directory: ' . (file_exists($directory) ? 'Not a directory' : 'No such file or directory'));
        }
        $entries = [];
        while (($e = readdir($h)) !== false) {
            $entries[] = $e;
        }
        closedir($h);
        $this->__phpx_entries = $this->__phpx_filter($entries);
        $this->__phpx_pos = 0;
        $this->__phpx_sync();
    }

    /** @internal */
    protected function __phpx_filter(array $entries): array { return $entries; }

    /** @internal */
    protected function __phpx_sync(): void
    {
        $e = $this->__phpx_entries[$this->__phpx_pos] ?? '';
        $this->__phpx_setPath($e === '' ? $this->__phpx_dir : $this->__phpx_dir . '/' . $e);
    }

    public function getFilename(): string { return $this->__phpx_entries[$this->__phpx_pos] ?? ''; }

    public function getPath(): string { return $this->__phpx_dir; }

    public function getPathname(): string
    {
        $e = $this->__phpx_entries[$this->__phpx_pos] ?? '';
        return $e === '' ? '' : $this->__phpx_dir . '/' . $e;
    }

    public function isDot(): bool
    {
        $e = $this->getFilename();
        return $e === '.' || $e === '..';
    }

    public function current(): mixed { return $this; }
    public function key(): mixed { return $this->__phpx_pos; }

    public function next(): void
    {
        $this->__phpx_pos++;
        $this->__phpx_sync();
    }

    public function rewind(): void
    {
        $this->__phpx_pos = 0;
        $this->__phpx_sync();
    }

    public function valid(): bool { return $this->__phpx_pos < count($this->__phpx_entries); }

    public function seek(int $offset): void
    {
        if ($offset < 0 || $offset >= count($this->__phpx_entries)) {
            throw new OutOfBoundsException('Seek position ' . $offset . ' is out of range');
        }
        $this->__phpx_pos = $offset;
        $this->__phpx_sync();
    }

    public function __toString(): string { return $this->getFilename(); }
}

class FilesystemIterator extends DirectoryIterator
{
    const CURRENT_MODE_MASK = 240;
    const CURRENT_AS_PATHNAME = 32;
    const CURRENT_AS_FILEINFO = 0;
    const CURRENT_AS_SELF = 16;
    const KEY_MODE_MASK = 3840;
    const KEY_AS_PATHNAME = 0;
    const FOLLOW_SYMLINKS = 16384;
    const KEY_AS_FILENAME = 256;
    const NEW_CURRENT_AND_KEY = 256;
    const OTHER_MODE_MASK = 28672;
    const SKIP_DOTS = 4096;
    const UNIX_PATHS = 8192;

    /** @internal */
    protected int $__phpx_flags = 0;

    public function __construct(string $directory, int $flags = FilesystemIterator::KEY_AS_PATHNAME | FilesystemIterator::CURRENT_AS_FILEINFO | FilesystemIterator::SKIP_DOTS)
    {
        $this->__phpx_flags = $flags | self::SKIP_DOTS;
        if ($directory === '') {
            throw new ValueError(static::class . '::__construct(): Argument #1 ($directory) cannot be empty');
        }
        $this->__phpx_open($directory);
    }

    /** @internal */
    protected function __phpx_filter(array $entries): array
    {
        if ($this->__phpx_flags & self::SKIP_DOTS) {
            $entries = array_values(array_filter($entries, fn($e) => $e !== '.' && $e !== '..'));
        }
        return $entries;
    }

    public function getFlags(): int { return $this->__phpx_flags & (self::KEY_MODE_MASK | self::CURRENT_MODE_MASK | self::OTHER_MODE_MASK); }
    public function setFlags(int $flags): void { $this->__phpx_flags = $flags; }

    public function current(): mixed
    {
        if ($this->__phpx_flags & self::CURRENT_AS_PATHNAME) {
            return $this->getPathname();
        }
        if ($this->__phpx_flags & self::CURRENT_AS_SELF) {
            return $this;
        }
        return $this->getFileInfo();
    }

    public function key(): mixed
    {
        return ($this->__phpx_flags & self::KEY_AS_FILENAME) ? $this->getFilename() : $this->getPathname();
    }
}

class RecursiveDirectoryIterator extends FilesystemIterator implements RecursiveIterator
{
    /** @internal */
    protected string $__phpx_sub = '';

    public function __construct(string $directory, int $flags = FilesystemIterator::KEY_AS_PATHNAME | FilesystemIterator::CURRENT_AS_FILEINFO)
    {
        $this->__phpx_flags = $flags;
        if ($directory === '') {
            throw new ValueError(static::class . '::__construct(): Argument #1 ($directory) cannot be empty');
        }
        $this->__phpx_open($directory);
    }

    public function hasChildren(bool $allowLinks = false): bool
    {
        if ($this->isDot()) {
            return false;
        }
        $p = $this->getPathname();
        if (!$allowLinks && !($this->__phpx_flags & self::FOLLOW_SYMLINKS) && is_link($p)) {
            return false;
        }
        return is_dir($p);
    }

    public function getChildren(): RecursiveDirectoryIterator
    {
        $c = new static($this->getPathname(), $this->__phpx_flags);
        $c->__phpx_sub = ($this->__phpx_sub === '' ? '' : $this->__phpx_sub . '/') . $this->getFilename();
        return $c;
    }

    public function getSubPath(): string { return $this->__phpx_sub; }

    public function getSubPathname(): string
    {
        return ($this->__phpx_sub === '' ? '' : $this->__phpx_sub . '/') . $this->getFilename();
    }

    public function key(): mixed
    {
        return ($this->__phpx_flags & self::KEY_AS_FILENAME) ? $this->getFilename() : $this->getPathname();
    }
}

class GlobIterator extends FilesystemIterator implements Countable
{
    public function __construct(string $pattern, int $flags = FilesystemIterator::KEY_AS_PATHNAME | FilesystemIterator::CURRENT_AS_FILEINFO)
    {
        $this->__phpx_flags = $flags;
        $files = glob($pattern) ?: [];
        $i = strrpos($pattern, '/');
        $this->__phpx_dir = $i === false ? '.' : substr($pattern, 0, $i);
        $this->__phpx_entries = array_map('basename', $files);
        $this->__phpx_pos = 0;
        $this->__phpx_sync();
    }

    public function count(): int { return count($this->__phpx_entries); }
}

class SplFileObject extends SplFileInfo implements RecursiveIterator, SeekableIterator
{
    const DROP_NEW_LINE = 1;
    const READ_AHEAD = 2;
    const SKIP_EMPTY = 4;
    const READ_CSV = 8;

    /** @internal */
    protected $__phpx_h = null;
    /** @internal */
    protected int $__phpx_flags = 0;
    /** @internal */
    protected mixed $__phpx_line = null;
    /** @internal */
    protected int $__phpx_lineNo = 0;
    /** @internal */
    protected int $__phpx_phys = 0;
    /** @internal */
    protected array $__phpx_csv = [',', '"', '\\'];
    /** @internal */
    protected int $__phpx_maxLen = 0;

    public function __construct(string $filename, string $mode = 'r', bool $useIncludePath = false, $context = null)
    {
        parent::__construct($filename);
        $h = @fopen($filename, $mode, $useIncludePath);
        if ($h === false) {
            throw new RuntimeException('SplFileObject::__construct(' . $filename . '): Failed to open stream: ' . (file_exists($filename) ? 'Permission denied' : 'No such file or directory'));
        }
        $this->__phpx_h = $h;
    }

    /** @internal Read the next line into the current one (as PHP: past the
     * last line comes one empty line, then nothing). */
    protected function __phpx_readLine(): bool
    {
        while (true) {
            if (feof($this->__phpx_h)) {
                $this->__phpx_line = null;
                return false;
            }
            $raw = $this->__phpx_maxLen > 0 ? fgets($this->__phpx_h, $this->__phpx_maxLen + 1) : fgets($this->__phpx_h);
            $this->__phpx_lineNo = $this->__phpx_phys++;
            $line = $raw === false ? '' : $raw;
            if ($this->__phpx_flags & self::DROP_NEW_LINE) {
                if (str_ends_with($line, "\n")) {
                    $line = substr($line, 0, -1);
                }
                if (str_ends_with($line, "\r")) {
                    $line = substr($line, 0, -1);
                }
            }
            if (($this->__phpx_flags & self::SKIP_EMPTY) && ($line === '' || (($this->__phpx_flags & self::READ_CSV) && trim($line, "\r\n") === ''))) {
                continue;
            }
            $this->__phpx_line = ($this->__phpx_flags & self::READ_CSV) ? str_getcsv(rtrim($line, "\r\n"), ...$this->__phpx_csv) : $line;
            return true;
        }
    }

    public function rewind(): void
    {
        rewind($this->__phpx_h);
        $this->__phpx_phys = 0;
        $this->__phpx_lineNo = 0;
        $this->__phpx_line = null;
        if ($this->__phpx_flags & self::READ_AHEAD) {
            $this->__phpx_readLine();
        }
    }

    public function valid(): bool
    {
        if ($this->__phpx_flags & self::READ_AHEAD) {
            return $this->__phpx_line !== null;
        }
        return $this->__phpx_line !== null || !feof($this->__phpx_h);
    }

    public function current(): string|array|false
    {
        if ($this->__phpx_line === null && !$this->__phpx_readLine()) {
            return false;
        }
        return $this->__phpx_line;
    }

    public function key(): int { return $this->__phpx_lineNo; }

    public function next(): void
    {
        $this->__phpx_line = null;
        if ($this->__phpx_flags & self::READ_AHEAD) {
            $this->__phpx_readLine();
        } else {
            $this->__phpx_lineNo = $this->__phpx_phys;
        }
    }

    public function seek(int $line): void
    {
        if ($line < 0) {
            throw new ValueError('SplFileObject::seek(): Argument #1 ($line) must be greater than or equal to 0');
        }
        $this->rewind();
        for ($i = 0; $i < $line && $this->valid(); $i++) {
            $this->current();
            $this->next();
        }
    }

    public function eof(): bool { return feof($this->__phpx_h); }
    public function fgets(): string
    {
        $l = fgets($this->__phpx_h);
        $this->__phpx_phys++;
        return $l === false ? '' : $l;
    }
    public function fgetc(): string|false { return fgetc($this->__phpx_h); }
    public function fread(int $length): string|false { return fread($this->__phpx_h, $length); }
    public function fgetcsv(string $separator = ',', string $enclosure = '"', string $escape = '\\'): array|false { return fgetcsv($this->__phpx_h, null, $separator, $enclosure, $escape); }
    public function fputcsv(array $fields, string $separator = ',', string $enclosure = '"', string $escape = '\\', string $eol = "\n"): int|false { return fputcsv($this->__phpx_h, $fields, $separator, $enclosure, $escape, $eol); }
    public function setCsvControl(string $separator = ',', string $enclosure = '"', string $escape = '\\'): void { $this->__phpx_csv = [$separator, $enclosure, $escape]; }
    public function getCsvControl(): array { return $this->__phpx_csv; }
    public function flock(int $operation, &$wouldBlock = null): bool { return flock($this->__phpx_h, $operation); }
    public function fflush(): bool { return fflush($this->__phpx_h); }
    public function ftell(): int|false { return ftell($this->__phpx_h); }
    public function fseek(int $offset, int $whence = SEEK_SET): int { return fseek($this->__phpx_h, $offset, $whence); }
    public function fpassthru(): int { return fpassthru($this->__phpx_h); }
    public function fscanf(string $format, mixed &...$vars): array|int|null { return sscanf((string) fgets($this->__phpx_h), $format); }
    public function fwrite(string $data, int $length = 0): int|false { return $length > 0 ? fwrite($this->__phpx_h, $data, $length) : fwrite($this->__phpx_h, $data); }
    public function fstat(): array { return fstat($this->__phpx_h); }
    public function ftruncate(int $size): bool { return ftruncate($this->__phpx_h, $size); }
    public function setFlags(int $flags): void { $this->__phpx_flags = $flags; }
    public function getFlags(): int { return $this->__phpx_flags; }
    public function setMaxLineLen(int $maxLength): void { $this->__phpx_maxLen = $maxLength; }
    public function getMaxLineLen(): int { return $this->__phpx_maxLen; }
    public function hasChildren(): bool { return false; }
    public function getChildren(): ?RecursiveIterator { return null; }
    public function getCurrentLine(): string { return $this->fgets(); }
    public function __toString(): string { return (string) $this->current(); }
}

class SplTempFileObject extends SplFileObject
{
    public function __construct(int $maxMemory = 2097152)
    {
        $name = $maxMemory < 0 ? 'php://memory' : 'php://temp/maxmemory:' . $maxMemory;
        SplFileInfo::__construct($name);
        $this->__phpx_h = fopen($name, 'w+');
    }
}

class RegexIterator extends FilterIterator
{
    const USE_KEY = 1;
    const INVERT_MATCH = 2;
    const MATCH = 0;
    const GET_MATCH = 1;
    const ALL_MATCHES = 2;
    const SPLIT = 3;
    const REPLACE = 4;

    public ?string $replacement = null;
    private mixed $__phpx_current = null;

    public function __construct(Iterator $iterator, private string $pattern, private int $mode = self::MATCH, private int $flags = 0, private int $pregFlags = 0)
    {
        parent::__construct($iterator);
    }

    public function accept(): bool
    {
        $subject = (string) (($this->flags & self::USE_KEY) ? parent::key() : parent::current());
        $this->__phpx_current = null;
        $ok = match ($this->mode) {
            self::MATCH => preg_match($this->pattern, $subject) > 0,
            self::GET_MATCH => preg_match($this->pattern, $subject, $m, $this->pregFlags) > 0 && (($this->__phpx_current = $m) || true),
            self::ALL_MATCHES => (preg_match_all($this->pattern, $subject, $m, $this->pregFlags ?: PREG_PATTERN_ORDER) || true) && (($this->__phpx_current = $m) || true),
            self::SPLIT => count($m = preg_split($this->pattern, $subject, -1, $this->pregFlags)) > 1 && (($this->__phpx_current = $m) || true),
            self::REPLACE => ($r = preg_replace($this->pattern, (string) $this->replacement, $subject)) !== null && (($this->__phpx_current = $r) || true),
            default => false,
        };
        return ($this->flags & self::INVERT_MATCH) ? !$ok : $ok;
    }

    public function current(): mixed
    {
        return $this->__phpx_current ?? parent::current();
    }

    public function getMode(): int { return $this->mode; }
    public function setMode(int $mode): void { $this->mode = $mode; }
    public function getFlags(): int { return $this->flags; }
    public function setFlags(int $flags): void { $this->flags = $flags; }
    public function getPregFlags(): int { return $this->pregFlags; }
    public function setPregFlags(int $pregFlags): void { $this->pregFlags = $pregFlags; }
    public function getRegex(): string { return $this->pattern; }
}

class RecursiveRegexIterator extends RegexIterator implements RecursiveIterator
{
    public function __construct(RecursiveIterator $iterator, string $pattern, int $mode = self::MATCH, int $flags = 0, int $pregFlags = 0)
    {
        parent::__construct($iterator, $pattern, $mode, $flags, $pregFlags);
    }

    public function hasChildren(): bool { return $this->getInnerIterator()->hasChildren(); }

    public function getChildren(): RecursiveRegexIterator
    {
        return new static($this->getInnerIterator()->getChildren(), $this->getRegex(), $this->getMode(), $this->getFlags(), $this->getPregFlags());
    }

    public function accept(): bool
    {
        return $this->getInnerIterator()->hasChildren() || parent::accept();
    }
}

class MultipleIterator implements Iterator
{
    const MIT_NEED_ANY = 0;
    const MIT_NEED_ALL = 1;
    const MIT_KEYS_NUMERIC = 0;
    const MIT_KEYS_ASSOC = 2;

    private array $iterators = [];

    public function __construct(private int $flags = MultipleIterator::MIT_NEED_ALL | MultipleIterator::MIT_KEYS_NUMERIC) {}

    public function getFlags(): int { return $this->flags; }
    public function setFlags(int $flags): void { $this->flags = $flags; }

    public function attachIterator(Iterator $iterator, string|int|null $info = null): void
    {
        if (($this->flags & self::MIT_KEYS_ASSOC) && $info === null) {
            throw new InvalidArgumentException('Sub-Iterator is associated with NULL');
        }
        $this->iterators[] = [$iterator, $info];
    }

    public function detachIterator(Iterator $iterator): void
    {
        $this->iterators = array_values(array_filter($this->iterators, fn($p) => $p[0] !== $iterator));
    }

    public function containsIterator(Iterator $iterator): bool
    {
        foreach ($this->iterators as [$it]) {
            if ($it === $iterator) {
                return true;
            }
        }
        return false;
    }

    public function countIterators(): int { return count($this->iterators); }

    public function rewind(): void
    {
        foreach ($this->iterators as [$it]) {
            $it->rewind();
        }
    }

    public function valid(): bool
    {
        if (!$this->iterators) {
            return false;
        }
        $any = false;
        $all = true;
        foreach ($this->iterators as [$it]) {
            $v = $it->valid();
            $any = $any || $v;
            $all = $all && $v;
        }
        return ($this->flags & self::MIT_NEED_ALL) ? $all : $any;
    }

    public function next(): void
    {
        foreach ($this->iterators as [$it]) {
            $it->next();
        }
    }

    private function collect(bool $keys): array
    {
        $out = [];
        foreach ($this->iterators as $i => [$it, $info]) {
            $v = $it->valid() ? ($keys ? $it->key() : $it->current()) : null;
            if (!$it->valid() && ($this->flags & self::MIT_NEED_ALL)) {
                throw new RuntimeException('Called ' . ($keys ? 'key' : 'current') . '() with non valid sub iterator');
            }
            if ($this->flags & self::MIT_KEYS_ASSOC) {
                $out[$info] = $v;
            } else {
                $out[$i] = $v;
            }
        }
        return $out;
    }

    public function current(): array { return $this->collect(false); }
    public function key(): array { return $this->collect(true); }
}
