
// ---- ext/random (natives in crates/interp/src/random.rs) ---------------------------------

namespace Random {
    interface Engine
    {
        public function generate(): string;
    }

    interface CryptoSafeEngine extends Engine
    {
    }

    class RandomError extends \Error
    {
    }

    class BrokenRandomEngineError extends RandomError
    {
    }

    class RandomException extends \Exception
    {
    }

    enum IntervalBoundary
    {
        case ClosedOpen;
        case ClosedClosed;
        case OpenClosed;
        case OpenOpen;
    }

    final class Randomizer
    {
        public readonly Engine $engine;

        public function __construct(?Engine $engine = null)
        {
            $this->engine = $engine ?? new Engine\Secure();
        }

        public function nextInt(): int {}
        public function nextFloat(): float {}
        public function getFloat(float $min, float $max, IntervalBoundary $boundary = IntervalBoundary::ClosedOpen): float {}
        public function getInt(int $min, int $max): int {}
        public function getBytes(int $length): string {}
        public function getBytesFromString(string $string, int $length): string {}
        public function shuffleArray(array $array): array {}
        public function shuffleBytes(string $bytes): string {}
        public function pickArrayKeys(array $array, int $num): array {}

        public function __serialize(): array
        {
            return [['engine' => $this->engine]];
        }

        public function __unserialize(array $data): void
        {
            if (count($data) !== 1 || !is_array($data[0] ?? null) || !(($data[0]['engine'] ?? null) instanceof Engine)) {
                throw new \Exception('Invalid serialization data for Random\Randomizer object');
            }
            $this->engine = $data[0]['engine'];
        }
    }
}

namespace Random\Engine {
    final class Mt19937 implements \Random\Engine
    {
        public function __construct(int|null $seed = null, int $mode = MT_RAND_MT19937) {}
        public function generate(): string {}
    }

    final class PcgOneseq128XslRr64 implements \Random\Engine
    {
        public function __construct(string|int|null $seed = null) {}
        public function generate(): string {}
        public function jump(int $advance): void {}
    }

    final class Xoshiro256StarStar implements \Random\Engine
    {
        public function __construct(string|int|null $seed = null) {}
        public function generate(): string {}
        public function jump(): void {}
        public function jumpLong(): void {}
    }

    final class Secure implements \Random\CryptoSafeEngine
    {
        public function generate(): string {}
    }
}
