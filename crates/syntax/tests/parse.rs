use phpx_syntax::ast::*;
use phpx_syntax::{check, Diagnostic};

fn codes(src: &str) -> Vec<&'static str> {
    check(src).1.iter().map(|d| d.code).collect()
}

fn ok(src: &str) -> File {
    let (f, d) = check(src);
    let errors: Vec<&Diagnostic> = d.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:#?}");
    f
}

#[test]
fn laravel_style_controller_parses() {
    let f = ok(r#"<?php
declare(strict_types=1);

namespace App\Http\Controllers;

use App\Models\{User, Post};
use Illuminate\Support\Facades\DB;
use function Illuminate\Support\defer;

#[Middleware('auth')]
final class ProfileController extends Controller implements HasMiddleware
{
    use AuthorizesRequests, ValidatesRequests;

    public const int PER_PAGE = 20;

    public function __construct(
        private readonly UserRepository $users,
        protected ?Cache $cache = null,
    ) {}

    /** @return array<string, mixed> */
    public function show(int $id): Response|array
    {
        $user = $this->users->find($id) ?? throw new NotFoundException("user {$id}");
        [$posts, $friends] = Concurrency::run([
            fn() => Http::get("/posts/{$user->ext_id}")->json(),
            fn() => DB::table('friends')->where('user_id', $user->id)->get(),
        ]);
        defer(fn() => Analytics::track('profile', $id));
        return match (true) {
            $posts === [] => view('empty'),
            default => view('profile', compact('user', 'posts', 'friends')),
        };
    }
}

enum Status: string implements HasLabel
{
    case Active = 'active';
    case Banned = 'banned';

    public function label(): string
    {
        return ucfirst($this->value);
    }
}
"#);
    let classes: Vec<&Class> = f
        .stmts
        .iter()
        .filter_map(|s| match &s.kind {
            StmtKind::Class(c) => Some(&**c),
            _ => None,
        })
        .collect();
    assert_eq!(classes.len(), 2);
    assert_eq!(classes[0].name.as_ref().unwrap().name, "ProfileController");
    assert_eq!(classes[0].attrs[0].name, "Middleware");
    let Member::Method { func, .. } = &classes[0].members[2] else { panic!() };
    assert_eq!(func.params.len(), 2);
    assert!(func.params[0].promote.as_ref().unwrap().readonly);
    let Member::Method { func, .. } = &classes[0].members[3] else { panic!() };
    assert_eq!(func.doc.as_deref(), Some("/** @return array<string, mixed> */"));
    assert_eq!(classes[1].kind, ClassKind::Enum);
}

#[test]
fn templates_and_alt_syntax() {
    ok(r#"<ul>
<?php foreach ($items as $i => $item): ?>
  <li class="<?= $i % 2 ? 'odd' : 'even' ?>"><?= e($item->name) ?></li>
<?php endforeach; ?>
</ul>
<?php if ($x): ?>a<?php elseif ($y): ?>b<?php else: ?>c<?php endif ?>
"#);
}

#[test]
fn string_interpolation() {
    let f = ok(r#"<?php $s = "Hi {$u->name}, you have $n[0] new {$counts['msgs']} \u{1F600}\n";"#);
    let StmtKind::Expr(Expr { kind: ExprKind::Assign { value, .. }, .. }) = &f.stmts[0].kind else { panic!() };
    let ExprKind::Interp(parts) = &value.kind else { panic!("{value:?}") };
    assert_eq!(parts.len(), 7);
    assert_eq!(parts[0], InterpPart::Lit("Hi ".into()));
    assert!(matches!(&parts[1], InterpPart::Expr(Expr { kind: ExprKind::Prop { .. }, .. })));
    assert!(matches!(&parts[3], InterpPart::Expr(Expr { kind: ExprKind::Index { .. }, .. })));
    assert_eq!(parts[6], InterpPart::Lit(" \u{1F600}\n".into()));
}

#[test]
fn rejects_dropped_features_with_codes() {
    assert_eq!(codes("<?php f(&$x);"), ["E0002"]); // call-time by-ref, removed in PHP 5.4
    assert_eq!(codes("<?php $x = `ls`;"), ["E1006"]);
    assert_eq!(codes("<?php create_function('$a', 'return 1;');"), ["E1007"]);
    assert_eq!(codes("<?php class A { public int $x { get => 1; } }"), ["E1008"]);
}

#[test]
fn accepts_dynamic_scope_features() {
    // WordPress, Drupal and friends depend on these.
    for src in [
        "<?php eval('1;');",
        "<?php $$name = 1; $$$deeper = 2; echo $$a['k'];",
        "<?php echo ${'x'}, \"${x} ${x['k']} ${'a' . 'b'}\";",
        "<?php function f() { global $db, $$other; }",
        "<?php a: goto a;",
        "<?php extract($data, EXTR_SKIP);",
        "<?php echo $GLOBALS['x']; $GLOBALS['y'] = 1; unset($GLOBALS['z']);",
        "<?php @unlink($f);",
    ] {
        assert_eq!(codes(src), Vec::<&str>::new(), "{src}");
    }
}

#[test]
fn allowed_lookalikes_are_not_rejected() {
    // Dynamic member access is dispatch, not variable variables.
    ok("<?php $obj->$prop = $obj->{$name . 'Id'}($x); Foo::$method(); $cls::create();");
    // Bitwise AND and intersection types are not references.
    ok("<?php $m = $a & $b; function f(A&B $x): void {}");
    // References are supported (Laravel's core depends on them).
    ok("<?php $n = 0; $q->chunk(100, function ($rows) use (&$n) { $n += count($rows); });");
    ok("<?php function &f(array &$a, &...$rest) { foreach ($a as $k => &$v) { $v++; } $x = &$a[0]; $y = [&$x]; return $a; }");
    // Reading the local scope by name is compilable; only writing by name isn't.
    ok("<?php return compact('a', ['b', 'c']) + compact($names) + get_defined_vars();");
    // `enum` and `match` are still usable as plain names.
    ok("<?php enum(1); $x->match(2); const ENUM = 3;");
}

#[test]
fn recovers_and_reports_every_error() {
    let (_, d) = check("<?php\n$a = ;\n$c = `ls`;\n$b = 1 2;\nfunction ok() { return 1; }\n");
    let codes: Vec<_> = d.iter().map(|d| d.code).collect();
    assert_eq!(codes, ["E0002", "E1006", "E0002"], "{d:#?}");
}

#[test]
fn diagnostic_renders_with_caret() {
    let src = "<?php\nfunction f() { $x = `ls`; }\n";
    let (_, d) = check(src);
    let out = d[0].render("app/f.php", src);
    assert_eq!(
        out,
        "error[E1006]: backtick shell execution is not supported\n \
         --> app/f.php:2:21\n  \
         |\n\
         2 | function f() { $x = `ls`; }\n  \
         |                     ^^^^\n  \
         = help: use Process::run() (or shell_exec()) explicitly\n"
    );
}
