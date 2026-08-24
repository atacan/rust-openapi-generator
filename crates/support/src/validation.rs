//! Bucket-2 runtime validation primitives (companion §9, DECISIONS.md
//! D-§2 bucket 2, D-impl-runtime-validation-timing Phase 2 half).
//!
//! Generated models carry `validate_request()` methods (and constrained
//! scalar aliases carry free `validate_<name>_request` functions) that call
//! into this module. The generated routers run them after a successful
//! decode of any server request body and map every [`Violation`] onto the
//! main spec §39 `SchemaViolation` → 422 path; client decoding stays lenient
//! (companion §9 default) and never calls validators.
//!
//! # Lenient-skip policy for undecidable patterns (loudly documented)
//!
//! Pattern constraints are evaluated by a bounded backtracking matcher over
//! the common ECMA-262 subset ([`evaluate_pattern`]). Two conditions make a
//! pattern UNDECIDABLE for this engine:
//!
//! 1. it uses constructs outside the supported subset (lookaround,
//!    backreferences, flags, …), or
//! 2. matching exceeded the hard step budget ([`PATTERN_STEP_BUDGET`]),
//!    which bounds worst-case work and makes catastrophic backtracking
//!    (ReDoS) impossible — the matcher aborts instead of hanging.
//!
//! An undecidable pattern is **not evidence of a violation**, so
//! [`validate_string`] skips exactly that constraint and returns `Ok(())`
//! (lenient skip). Rejecting on undecidability would 422 valid documents,
//! and these dependency-free primitives have no logging hook through which a
//! pass-with-diagnostic could surface. The standalone [`evaluate_pattern`]
//! returns [`PatternDecision::Unsupported`] so tests and future strict modes
//! can observe undecidability directly, and [`Violation::PatternUnsupported`]
//! remains the explicit variant for engines that choose strictness.
//!
//! # Documented v1 limits
//!
//! - numeric comparisons go through `f64`; integers beyond ±2^53 may lose
//!   precision at the boundaries;
//! - `multipleOf` uses an epsilon-tolerant quotient test because binary
//!   floating point cannot represent decimal divisors exactly;
//! - `uniqueItems` is enforced only for string- and number-typed elements
//!   ([`require_unique_strings`] / [`require_unique_numbers`]); other element
//!   types skip the check;
//! - unknown `format` names are ignored ([`validate_format_string`] returns
//!   `Ok`); the recognized set is intentionally small.

use std::collections::HashSet;

/// Hard upper bound on VM steps for one pattern evaluation. Attempts that
/// exceed it abort with [`PatternDecision::Unsupported`] instead of hanging
/// (ReDoS-safe by construction).
pub const PATTERN_STEP_BUDGET: u32 = 10_000;

/// One bucket-2 constraint failure with expected/actual detail.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Violation {
    /// Value failed its `pattern`.
    #[error("value {actual:?} does not match pattern {pattern:?}")]
    Pattern {
        /// The wire pattern.
        pattern: String,
        /// The rejected value.
        actual: String,
    },
    /// String shorter than `minLength`.
    #[error("length {actual} is below minLength {expected}")]
    MinLength {
        /// Required minimum.
        expected: u64,
        /// Actual character length.
        actual: usize,
    },
    /// String longer than `maxLength`.
    #[error("length {actual} exceeds maxLength {expected}")]
    MaxLength {
        /// Allowed maximum.
        expected: u64,
        /// Actual character length.
        actual: usize,
    },
    /// Number below `minimum` (`exclusive == false`) or `exclusiveMinimum`
    /// (`exclusive == true`).
    #[error("value {actual} violates {} {expected}", if *.exclusive { "exclusiveMinimum" } else { "minimum" })]
    Minimum {
        /// Bound value.
        expected: f64,
        /// True when the bound itself is excluded.
        exclusive: bool,
        /// Actual value.
        actual: f64,
    },
    /// Number above `maximum` (`exclusive == false`) or `exclusiveMaximum`
    /// (`exclusive == true`).
    #[error("value {actual} violates {} {expected}", if *.exclusive { "exclusiveMaximum" } else { "maximum" })]
    Maximum {
        /// Bound value.
        expected: f64,
        /// True when the bound itself is excluded.
        exclusive: bool,
        /// Actual value.
        actual: f64,
    },
    /// Number is not a multiple of `multipleOf`.
    #[error("value is not a multiple of multipleOf {expected}")]
    MultipleOf {
        /// Divisor.
        expected: f64,
    },
    /// Array shorter than `minItems`.
    #[error("item count {actual} is below minItems {expected}")]
    MinItems {
        /// Required minimum.
        expected: u64,
        /// Actual item count.
        actual: usize,
    },
    /// Array longer than `maxItems`.
    #[error("item count {actual} exceeds maxItems {expected}")]
    MaxItems {
        /// Allowed maximum.
        expected: u64,
        /// Actual item count.
        actual: usize,
    },
    /// Array elements were not unique.
    #[error("array items are not unique")]
    UniqueItems,
    /// Fewer items than `minContains` matched the `contains` schema.
    #[error("{actual} item(s) match `contains`, below minContains {expected}")]
    ContainsMin {
        /// Required matching-item minimum.
        expected: u64,
        /// Actual matching-item count.
        actual: usize,
    },
    /// More items than `maxContains` matched the `contains` schema.
    #[error("{actual} item(s) match `contains`, above maxContains {expected}")]
    ContainsMax {
        /// Allowed matching-item maximum.
        expected: u64,
        /// Actual matching-item count.
        actual: usize,
    },
    /// Object had fewer than `minProperties` properties.
    #[error("property count {actual} is below minProperties {expected}")]
    MinProperties {
        /// Required minimum.
        expected: u64,
        /// Actual property count.
        actual: usize,
    },
    /// Object had more than `maxProperties` properties.
    #[error("property count {actual} exceeds maxProperties {expected}")]
    MaxProperties {
        /// Allowed maximum.
        expected: u64,
        /// Actual property count.
        actual: usize,
    },
    /// Value failed its declared `format`.
    #[error("value {actual:?} is not a valid {format}")]
    Format {
        /// Declared format name.
        format: String,
        /// The rejected value.
        actual: String,
    },
    /// The pattern could not be decided (unsupported construct or step
    /// budget exhausted). Never produced by the lenient `validate_*`
    /// functions; see the module-level policy note.
    #[error("pattern {pattern:?} is unsupported for bounded matching")]
    PatternUnsupported {
        /// The pattern that could not be decided.
        pattern: String,
    },
    /// A nested violation annotated with the object field (or array element
    /// family, or choice branch) it originated from. Generated validators
    /// wrap every constraint failure so rejections name the offending
    /// location (companion §9); nesting composes into a path.
    #[error("field `{field}`: {source}")]
    Field {
        /// Field name as emitted in the generated model (snake_case Rust
        /// identifier, `[*]` suffix for array elements).
        field: String,
        /// The wrapped constraint failure.
        #[source]
        source: Box<Violation>,
    },
}

impl Violation {
    /// Annotates this violation with the field it came from (companion §9);
    /// repeated calls compose outermost-last into a path.
    #[must_use]
    pub fn at_field(self, field: impl Into<String>) -> Self {
        Self::Field {
            field: field.into(),
            source: Box::new(self),
        }
    }

    /// The innermost non-[`Violation::Field`] violation.
    #[must_use]
    pub fn innermost(&self) -> &Self {
        let mut current = self;
        while let Self::Field { source, .. } = current {
            current = source;
        }
        current
    }
}

/// Attaches `field` to a failing validation result — the statement-level
/// form generated validators use, keeping emitted code free of
/// multi-method chains.
///
/// # Errors
///
/// Passes `Ok` through untouched; wraps `Err` via [`Violation::at_field`].
pub fn located(field: impl Into<String>, result: Result<(), Violation>) -> Result<(), Violation> {
    result.map_err(|violation| violation.at_field(field))
}

/// String constraints mirrored 1:1 from the IR's `ValidationMeta` so emitted
/// literals map field-for-field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StringConstraints<'a> {
    /// `pattern` as written in the document.
    pub pattern: Option<&'a str>,
    /// `minLength` in characters.
    pub min_length: Option<u64>,
    /// `maxLength` in characters.
    pub max_length: Option<u64>,
}

/// Array length constraints (`uniqueItems` rides separately through the
/// typed uniqueness helpers because element types decide the algorithm).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArrayConstraints {
    /// `minItems`.
    pub min_items: Option<u64>,
    /// `maxItems`.
    pub max_items: Option<u64>,
}

/// Validates one string against pattern/length constraints.
///
/// Undecidable patterns are SKIPPED (lenient; module docs explain why).
///
/// # Errors
///
/// Returns the first violated constraint in pattern → minLength → maxLength
/// order.
pub fn validate_string(value: &str, constraints: &StringConstraints<'_>) -> Result<(), Violation> {
    if let Some(pattern) = constraints.pattern {
        // Lenient skip: an undecidable pattern is not evidence of violation.
        if matches!(evaluate_pattern(pattern, value), PatternDecision::NoMatch) {
            return Err(Violation::Pattern {
                pattern: pattern.to_owned(),
                actual: value.to_owned(),
            });
        }
    }
    if let Some(min_length) = constraints.min_length {
        let length = value.chars().count();
        if (length as u64) < min_length {
            return Err(Violation::MinLength {
                expected: min_length,
                actual: length,
            });
        }
    }
    if let Some(max_length) = constraints.max_length {
        let length = value.chars().count();
        if (length as u64) > max_length {
            return Err(Violation::MaxLength {
                expected: max_length,
                actual: length,
            });
        }
    }
    Ok(())
}

/// Validates one number against inclusive/exclusive bounds and `multipleOf`.
///
/// A non-positive or non-finite divisor is ignored (an invalid schema cannot
/// be expressed as a meaningful runtime check).
///
/// # Errors
///
/// Returns the first violated bound in minimum → maximum → multipleOf order.
pub fn validate_number(
    value: f64,
    min: Option<(f64, bool)>,
    max: Option<(f64, bool)>,
    multiple_of: Option<f64>,
) -> Result<(), Violation> {
    if !value.is_finite() {
        // NaN/infinity never arises from JSON numbers; treat as violation.
        return Err(Violation::Maximum {
            expected: f64::INFINITY,
            exclusive: false,
            actual: value,
        });
    }
    if let Some((bound, exclusive)) = min {
        let violated = if exclusive {
            value <= bound
        } else {
            value < bound
        };
        if violated {
            return Err(Violation::Minimum {
                expected: bound,
                exclusive,
                actual: value,
            });
        }
    }
    if let Some((bound, exclusive)) = max {
        let violated = if exclusive {
            value >= bound
        } else {
            value > bound
        };
        if violated {
            return Err(Violation::Maximum {
                expected: bound,
                exclusive,
                actual: value,
            });
        }
    }
    if let Some(divisor) = multiple_of {
        if divisor.is_finite() && divisor > 0.0 && !is_multiple_of(value, divisor) {
            return Err(Violation::MultipleOf { expected: divisor });
        }
    }
    Ok(())
}

/// Epsilon-tolerant integrality test of `value / divisor` (binary floats
/// cannot represent decimal divisors such as 0.1 exactly).
fn is_multiple_of(value: f64, divisor: f64) -> bool {
    let quotient = value / divisor;
    let tolerance = 1e-9_f64.max(quotient.abs() * 1e-9);
    (quotient - quotient.round()).abs() <= tolerance
}

/// Validates an array length against `minItems`/`maxItems`.
///
/// # Errors
///
/// [`Violation::MinItems`] or [`Violation::MaxItems`].
pub fn validate_array_len(len: usize, constraints: &ArrayConstraints) -> Result<(), Violation> {
    if let Some(min_items) = constraints.min_items {
        if (len as u64) < min_items {
            return Err(Violation::MinItems {
                expected: min_items,
                actual: len,
            });
        }
    }
    if let Some(max_items) = constraints.max_items {
        if (len as u64) > max_items {
            return Err(Violation::MaxItems {
                expected: max_items,
                actual: len,
            });
        }
    }
    Ok(())
}

/// Validates the `contains` match count against `minContains`/`maxContains`
/// (`min_contains` defaults to 1 at the call site when the document omitted
/// it, per JSON Schema).
///
/// # Errors
///
/// [`Violation::ContainsMin`] or [`Violation::ContainsMax`].
pub fn validate_contains_count(
    matched: usize,
    min_contains: Option<u64>,
    max_contains: Option<u64>,
) -> Result<(), Violation> {
    if let Some(min_contains) = min_contains {
        if (matched as u64) < min_contains {
            return Err(Violation::ContainsMin {
                expected: min_contains,
                actual: matched,
            });
        }
    }
    if let Some(max_contains) = max_contains {
        if (matched as u64) > max_contains {
            return Err(Violation::ContainsMax {
                expected: max_contains,
                actual: matched,
            });
        }
    }
    Ok(())
}

/// Validates an object property count against `minProperties`/`maxProperties`.
///
/// # Errors
///
/// [`Violation::MinProperties`] or [`Violation::MaxProperties`].
pub fn validate_object_props(
    count: usize,
    min: Option<u64>,
    max: Option<u64>,
) -> Result<(), Violation> {
    if let Some(min) = min {
        if (count as u64) < min {
            return Err(Violation::MinProperties {
                expected: min,
                actual: count,
            });
        }
    }
    if let Some(max) = max {
        if (count as u64) > max {
            return Err(Violation::MaxProperties {
                expected: max,
                actual: count,
            });
        }
    }
    Ok(())
}

/// Enforces `uniqueItems` for string elements (v1 documented limit:
/// uniqueness is only decidable here for strings and numbers). Generated
/// code passes `&vec_of_strings` directly.
///
/// # Errors
///
/// [`Violation::UniqueItems`] when any duplicate exists.
pub fn require_unique_strings<I, S>(items: I) -> Result<(), Violation>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen: HashSet<&str> = HashSet::new();
    let owned: Vec<S> = items.into_iter().collect();
    seen.reserve(owned.len());
    for item in &owned {
        if !seen.insert(item.as_ref()) {
            return Err(Violation::UniqueItems);
        }
    }
    Ok(())
}

/// Enforces `uniqueItems` for numeric elements. `-0.0` and `0.0` compare
/// equal (they are the same JSON number); `NaN` cannot appear in JSON.
/// Generated code maps integer elements through `as f64` at the call site.
///
/// # Errors
///
/// [`Violation::UniqueItems`] when any duplicate exists.
pub fn require_unique_numbers<I>(items: I) -> Result<(), Violation>
where
    I: IntoIterator<Item = f64>,
{
    let values: Vec<f64> = items.into_iter().collect();
    let mut seen: HashSet<u64> = HashSet::with_capacity(values.len());
    for item in values {
        let normalized = if item == 0.0 { 0.0 } else { item };
        if !seen.insert(normalized.to_bits()) {
            return Err(Violation::UniqueItems);
        }
    }
    Ok(())
}

/// Outcome of evaluating one pattern against one input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternDecision {
    /// The input matches the pattern.
    Match,
    /// The input provably does not match.
    NoMatch,
    /// The pattern uses unsupported constructs or exceeded the step budget;
    /// no verdict is possible (lenient callers skip the constraint).
    Unsupported,
}

/// Evaluates `pattern` against `input` under the hard step budget.
#[must_use]
pub fn evaluate_pattern(pattern: &str, input: &str) -> PatternDecision {
    let program = match compile(pattern) {
        Ok(program) => program,
        Err(()) => return PatternDecision::Unsupported,
    };
    let chars: Vec<char> = input.chars().collect();
    let mut vm = Vm {
        program: &program,
        chars: &chars,
        slots: vec![0_usize; program.slots],
        steps: 0,
    };
    // Unanchored search per ECMA-262: try every start offset; a leading `^`
    // fails those branches cheaply through the start assertion.
    for start in 0..=chars.len() {
        match vm.run(start) {
            RunOutcome::Matched => return PatternDecision::Match,
            RunOutcome::Failed => {}
            RunOutcome::Aborted => return PatternDecision::Unsupported,
        }
    }
    PatternDecision::NoMatch
}

// ----------------------------------------------------------------------
// Bounded backtracking matcher (common ECMA-262 subset)
// ----------------------------------------------------------------------
//
// Compiled to a small instruction program executed by a backtracking VM
// with an explicit backtrack stack. Supported syntax:
//
// - literal characters; `.` (any char except \n \r U+2028 U+2029);
// - classes `[...]`: ranges, negation, literal escapes, `\d \w \s \D \W \S`;
// - escapes `\n \t \r \f \v \0 \xHH \uHHHH` and escaped punctuation;
// - shorthand classes `\d \D \w \W \s \S`;
// - anchors `^` `$`;
// - groups `(...)` and non-capturing `(?:...)`;
// - alternation `|`;
// - quantifiers `* + ? {n} {n,} {n,m}`, greedy or lazy (`?` suffix).
//
// Everything else (lookaround, backreferences, inline flags, word
// boundaries) compiles to `Unsupported`. A malformed `{` that is not a
// quantifier stays a literal brace, mirroring ECMA-262. Unbounded loops
// carry a progress guard so empty-body iterations terminate.

/// Control-flow targets are PC-RELATIVE deltas from the instruction AFTER
/// the branch, so body slices can be duplicated by quantifier expansion
/// without target fixups.
#[derive(Debug, Clone)]
enum Inst {
    Char(char),
    Any,
    Class(usize),
    Split(i32, i32),
    Jmp(i32),
    AssertStart,
    AssertEnd,
    SaveSlot(u16),
    AssertProgress(u16),
    Match,
}

#[derive(Debug)]
struct Program {
    insts: Vec<Inst>,
    classes: Vec<ClassSet>,
    slots: usize,
}

#[derive(Debug, Default)]
struct ClassSet {
    negated: bool,
    ranges: Vec<(char, char)>,
    /// Positive shorthand members (`\d` inside a class).
    kinds: Vec<Shorthand>,
    /// Negated shorthand members (`\D` inside a class): char must NOT be in
    /// the referenced positive set.
    negated_kinds: Vec<Shorthand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shorthand {
    Digit,
    Word,
    Space,
}

fn compile(pattern: &str) -> Result<Program, ()> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut compiler = Compiler {
        chars: &chars,
        pos: 0,
        insts: Vec::new(),
        classes: Vec::new(),
        slots: 0,
        quest_patches: Vec::new(),
    };
    compiler.alternation()?;
    if compiler.pos != chars.len() {
        // An unmatched ')' terminated the parse early.
        return Err(());
    }
    compiler.patch_quests();
    compiler.emit(Inst::Match);
    debug_assert!(compiler.quest_patches.is_empty(), "all quests patched");
    Ok(Program {
        insts: compiler.insts,
        classes: compiler.classes,
        slots: compiler.slots,
    })
}

/// Caps `{n,m}` expansion ({400} would otherwise emit 400 body copies).
const MAX_EXPANSION: u32 = 256;

struct Compiler<'a> {
    chars: &'a [char],
    pos: usize,
    insts: Vec<Inst>,
    classes: Vec<ClassSet>,
    slots: usize,
    /// Quest-style splits awaiting their common exit target (patched once
    /// the enclosing construct's end position is known).
    quest_patches: Vec<QuestPatch>,
}

struct QuestPatch {
    split: usize,
    body_start: usize,
    lazy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quantifier {
    Star,
    Plus,
    Quest,
    Counted(u32, Option<u32>),
}

impl Compiler<'_> {
    /// Relative delta for a jump placed at `at` targeting `target`.
    fn delta(at: usize, target: usize) -> i32 {
        (target as isize - at as isize - 1) as i32
    }

    fn patch_split(&mut self, at: usize, first: usize, second: usize) {
        self.insts[at] = Inst::Split(Self::delta(at, first), Self::delta(at, second));
    }

    fn patch_jmp(&mut self, at: usize, target: usize) {
        self.insts[at] = Inst::Jmp(Self::delta(at, target));
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn emit(&mut self, inst: Inst) -> usize {
        self.insts.push(inst);
        self.insts.len() - 1
    }

    /// Opens one quest split whose exit target resolves later.
    fn open_quest(&mut self, lazy: bool) {
        let split = self.emit(Inst::Split(0, 0));
        let body_start = self.insts.len();
        self.quest_patches.push(QuestPatch {
            split,
            body_start,
            lazy,
        });
    }

    fn patch_quests(&mut self) {
        let exit = self.insts.len();
        for patch in std::mem::take(&mut self.quest_patches) {
            if patch.lazy {
                self.patch_split(patch.split, exit, patch.body_start);
            } else {
                self.patch_split(patch.split, patch.body_start, exit);
            }
        }
    }

    fn alternation(&mut self) -> Result<(), ()> {
        let mut branch_start = self.insts.len();
        self.concatenation()?;
        if self.peek() != Some('|') {
            return Ok(());
        }
        let mut branches = vec![self.insts.split_off(branch_start)];
        while self.peek() == Some('|') {
            self.pos += 1;
            branch_start = self.insts.len();
            self.concatenation()?;
            branches.push(self.insts.split_off(branch_start));
        }
        self.emit_alternatives(branches);
        Ok(())
    }

    /// Emits `b0 | b1 | … | bn` as a right-nested split chain whose exits
    /// all jump past the last branch.
    fn emit_alternatives(&mut self, mut branches: Vec<Vec<Inst>>) {
        let Some(first) = branches.pop() else {
            return;
        };
        let split = self.emit(Inst::Split(0, 0));
        let body_start = self.insts.len();
        self.insts.extend(first);
        let jump = self.emit(Inst::Jmp(0));
        let rest_start = self.insts.len();
        self.patch_split(split, body_start, rest_start);
        if branches.is_empty() {
            self.patch_jmp(jump, rest_start);
        } else {
            self.emit_alternatives(branches);
            let end = self.insts.len();
            self.patch_jmp(jump, end);
        }
    }

    fn concatenation(&mut self) -> Result<(), ()> {
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                return Ok(());
            }
            self.quantified_atom()?;
        }
        Ok(())
    }

    fn quantified_atom(&mut self) -> Result<(), ()> {
        let atom_start = self.insts.len();
        self.atom()?;
        let quantifier = self.try_quantifier()?;
        let Some(quantifier) = quantifier else {
            return Ok(());
        };
        let lazy = if self.peek() == Some('?') {
            self.pos += 1;
            true
        } else {
            false
        };
        match quantifier {
            Quantifier::Quest => {
                // Insert the split IN FRONT of the atom's instructions;
                // nothing before `atom_start` can be affected, and every
                // control target here is PC-relative.
                let split_index = atom_start;
                self.insts.insert(split_index, Inst::Split(0, 0));
                let body_start = split_index + 1;
                let exit = self.insts.len();
                if lazy {
                    self.patch_split(split_index, exit, body_start);
                } else {
                    self.patch_split(split_index, body_start, exit);
                }
            }
            Quantifier::Star | Quantifier::Plus => {
                let body: Vec<Inst> = self.insts.drain(atom_start..).collect();
                if quantifier == Quantifier::Plus {
                    // One mandatory copy runs before the guarded loop.
                    self.insts.extend(body.iter().cloned());
                }
                self.emit_guarded_star(&body, lazy);
            }
            Quantifier::Counted(low, high) => {
                let body: Vec<Inst> = self.insts.drain(atom_start..).collect();
                if low > MAX_EXPANSION || high.is_some_and(|high| high > MAX_EXPANSION) {
                    return Err(());
                }
                for _ in 0..low {
                    self.insts.extend(body.iter().cloned());
                }
                match high {
                    None => self.emit_guarded_star(&body, lazy),
                    Some(high) => {
                        for _ in 0..(high - low) {
                            self.open_quest(lazy);
                            self.insts.extend(body.iter().cloned());
                        }
                        self.patch_quests();
                    }
                }
            }
        }
        Ok(())
    }

    /// Emits `L0: SaveSlot; Split(body, exit); body; AssertProgress; Jmp L0`
    /// (operand order flipped for lazy repeats). The progress guard makes
    /// empty-body iterations terminate instead of looping forever.
    fn emit_guarded_star(&mut self, body: &[Inst], lazy: bool) {
        let slot = self.slots as u16;
        self.slots += 1;
        let loop_head = self.emit(Inst::SaveSlot(slot));
        let split = self.emit(Inst::Split(0, 0));
        let body_start = self.insts.len();
        self.insts.extend_from_slice(body);
        self.emit(Inst::AssertProgress(slot));
        let jump_back = self.emit(Inst::Jmp(0));
        self.patch_jmp(jump_back, loop_head);
        let exit = self.insts.len();
        if lazy {
            self.patch_split(split, exit, body_start);
        } else {
            self.patch_split(split, body_start, exit);
        }
    }

    fn try_quantifier(&mut self) -> Result<Option<Quantifier>, ()> {
        match self.peek() {
            Some('*') => {
                self.pos += 1;
                Ok(Some(Quantifier::Star))
            }
            Some('+') => {
                self.pos += 1;
                Ok(Some(Quantifier::Plus))
            }
            Some('?') => {
                self.pos += 1;
                Ok(Some(Quantifier::Quest))
            }
            Some('{') => self.try_counted(),
            _ => Ok(None),
        }
    }

    /// Parses `{n}` / `{n,}` / `{n,m}`; a malformed brace falls back to a
    /// literal `{` (ECMA-262 Annex B behavior).
    fn try_counted(&mut self) -> Result<Option<Quantifier>, ()> {
        let saved = self.pos;
        self.pos += 1; // consume '{'
        let Some(low) = self.digits() else {
            self.pos = saved;
            return Ok(None);
        };
        match self.bump() {
            Some('}') => Ok(Some(Quantifier::Counted(low, Some(low)))),
            Some(',') => {
                let high = self.digits();
                if self.bump() != Some('}') {
                    self.pos = saved;
                    return Ok(None);
                }
                match high {
                    None => Ok(Some(Quantifier::Counted(low, None))),
                    Some(high) if high < low => Err(()),
                    Some(high) => Ok(Some(Quantifier::Counted(low, Some(high)))),
                }
            }
            _ => {
                self.pos = saved;
                Ok(None)
            }
        }
    }

    fn digits(&mut self) -> Option<u32> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse().ok()
    }

    fn atom(&mut self) -> Result<(), ()> {
        match self.bump().ok_or(())? {
            '^' => {
                self.emit(Inst::AssertStart);
                Ok(())
            }
            '$' => {
                self.emit(Inst::AssertEnd);
                Ok(())
            }
            '.' => {
                self.emit(Inst::Any);
                Ok(())
            }
            '[' => self.class(),
            '(' => self.group(),
            '\\' => self.escape_atom(),
            '*' | '+' | '?' => Err(()), // quantifier with nothing to repeat
            other => {
                self.emit(Inst::Char(other));
                Ok(())
            }
        }
    }

    fn group(&mut self) -> Result<(), ()> {
        if self.peek() == Some('?') {
            self.pos += 1;
            match self.bump() {
                Some(':') => {}      // non-capturing group
                _ => return Err(()), // lookaround / flags / named groups unsupported
            }
        }
        self.alternation()?;
        if self.bump() != Some(')') {
            return Err(());
        }
        Ok(())
    }

    fn escape_atom(&mut self) -> Result<(), ()> {
        let escape = self.bump().ok_or(())?;
        match escape {
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                let class = ClassSet {
                    negated: escape.is_ascii_uppercase(),
                    kinds: vec![shorthand_of(escape)],
                    ..ClassSet::default()
                };
                self.push_class(class);
                Ok(())
            }
            'b' | 'B' => Err(()), // word boundaries unsupported
            '1'..='9' => Err(()), // backreferences unsupported
            control @ ('n' | 't' | 'r' | 'f' | 'v' | '0') => {
                self.emit(Inst::Char(control_char(control)));
                Ok(())
            }
            'x' => {
                let hi = self.hex_digit().ok_or(())?;
                let lo = self.hex_digit().ok_or(())?;
                self.emit(Inst::Char(
                    char::from_u32(u32::from(hi) * 16 + u32::from(lo)).ok_or(())?,
                ));
                Ok(())
            }
            'u' => {
                let mut code = 0_u32;
                for _ in 0..4 {
                    code = code * 16 + u32::from(self.hex_digit().ok_or(())?);
                }
                self.emit(Inst::Char(char::from_u32(code).ok_or(())?));
                Ok(())
            }
            literal if literal.is_ascii_alphanumeric() => Err(()),
            literal => {
                self.emit(Inst::Char(literal));
                Ok(())
            }
        }
    }

    fn hex_digit(&mut self) -> Option<u8> {
        let ch = self.bump()?;
        ch.to_digit(16).map(|value| value as u8)
    }

    fn push_class(&mut self, class: ClassSet) {
        let index = self.classes.len();
        self.classes.push(class);
        self.emit(Inst::Class(index));
    }

    fn class(&mut self) -> Result<(), ()> {
        let mut set = ClassSet::default();
        if self.peek() == Some('^') {
            set.negated = true;
            self.pos += 1;
        }
        let mut first = true;
        loop {
            let ch = self.bump().ok_or(())?;
            if ch == ']' && !first {
                break;
            }
            first = false;
            let low = if ch == '\\' {
                match self.class_escape()? {
                    ClassPiece::Char(value) => value,
                    ClassPiece::Kind(kind, negated) => {
                        if negated {
                            set.negated_kinds.push(kind);
                        } else {
                            set.kinds.push(kind);
                        }
                        continue;
                    }
                }
            } else {
                ch
            };
            // Range? A trailing '-' before ']' stays a literal member.
            if self.peek() == Some('-')
                && self
                    .chars
                    .get(self.pos + 1)
                    .is_some_and(|next| *next != ']')
            {
                self.pos += 1; // '-'
                let next_ch = self.bump().ok_or(())?;
                let high = if next_ch == '\\' {
                    match self.class_escape()? {
                        ClassPiece::Char(value) => value,
                        ClassPiece::Kind(..) => return Err(()),
                    }
                } else {
                    next_ch
                };
                if high < low {
                    return Err(());
                }
                set.ranges.push((low, high));
            } else {
                set.ranges.push((low, low));
            }
        }
        self.push_class(set);
        Ok(())
    }

    fn class_escape(&mut self) -> Result<ClassPiece, ()> {
        let escape = self.bump().ok_or(())?;
        match escape {
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => Ok(ClassPiece::Kind(
                shorthand_of(escape),
                escape.is_ascii_uppercase(),
            )),
            'b' => Ok(ClassPiece::Char('\u{8}')),
            control @ ('n' | 't' | 'r' | 'f' | 'v' | '0') => {
                Ok(ClassPiece::Char(control_char(control)))
            }
            'x' => {
                let hi = self.hex_digit().ok_or(())?;
                let lo = self.hex_digit().ok_or(())?;
                Ok(ClassPiece::Char(
                    char::from_u32(u32::from(hi) * 16 + u32::from(lo)).ok_or(())?,
                ))
            }
            'u' => {
                let mut code = 0_u32;
                for _ in 0..4 {
                    code = code * 16 + u32::from(self.hex_digit().ok_or(())?);
                }
                Ok(ClassPiece::Char(char::from_u32(code).ok_or(())?))
            }
            literal if literal.is_ascii_alphanumeric() => Err(()),
            literal => Ok(ClassPiece::Char(literal)),
        }
    }
}

enum ClassPiece {
    Char(char),
    Kind(Shorthand, bool),
}

fn shorthand_of(escape: char) -> Shorthand {
    match escape.to_ascii_lowercase() {
        'd' => Shorthand::Digit,
        'w' => Shorthand::Word,
        _ => Shorthand::Space,
    }
}

fn control_char(control: char) -> char {
    match control {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        'f' => '\u{c}',
        'v' => '\u{b}',
        _ => '\0',
    }
}

enum RunOutcome {
    Matched,
    Failed,
    Aborted,
}

struct Vm<'a> {
    program: &'a Program,
    chars: &'a [char],
    slots: Vec<usize>,
    steps: u32,
}

impl Vm<'_> {
    fn run(&mut self, start: usize) -> RunOutcome {
        let mut stack: Vec<(usize, usize)> = vec![(0, start)];
        'backtrack: while let Some((mut pc, mut pos)) = stack.pop() {
            loop {
                self.steps += 1;
                if self.steps > PATTERN_STEP_BUDGET {
                    return RunOutcome::Aborted;
                }
                match &self.program.insts[pc] {
                    Inst::Char(expected) => {
                        if self.chars.get(pos) == Some(expected) {
                            pc += 1;
                            pos += 1;
                        } else {
                            continue 'backtrack;
                        }
                    }
                    Inst::Any => match self.chars.get(pos) {
                        Some(ch) if !matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') => {
                            pc += 1;
                            pos += 1;
                        }
                        _ => continue 'backtrack,
                    },
                    Inst::Class(index) => {
                        let matches = self
                            .chars
                            .get(pos)
                            .is_some_and(|ch| self.program.classes[*index].matches(*ch));
                        if matches {
                            pc += 1;
                            pos += 1;
                        } else {
                            continue 'backtrack;
                        }
                    }
                    Inst::Split(first, second) => {
                        let here = pc as isize;
                        stack.push(((here + 1 + *second as isize) as usize, pos));
                        pc = (here + 1 + *first as isize) as usize;
                    }
                    Inst::Jmp(delta) => pc = (pc as isize + 1 + *delta as isize) as usize,
                    Inst::AssertStart => {
                        if pos == 0 {
                            pc += 1;
                        } else {
                            continue 'backtrack;
                        }
                    }
                    Inst::AssertEnd => {
                        if pos == self.chars.len() {
                            pc += 1;
                        } else {
                            continue 'backtrack;
                        }
                    }
                    Inst::SaveSlot(slot) => {
                        self.slots[*slot as usize] = pos;
                        pc += 1;
                    }
                    Inst::AssertProgress(slot) => {
                        if pos != self.slots[*slot as usize] {
                            pc += 1;
                        } else {
                            continue 'backtrack;
                        }
                    }
                    Inst::Match => return RunOutcome::Matched,
                }
            }
        }
        RunOutcome::Failed
    }
}

impl ClassSet {
    fn matches(&self, ch: char) -> bool {
        let hit = self
            .ranges
            .iter()
            .any(|(low, high)| *low <= ch && ch <= *high)
            || self.kinds.iter().any(|kind| kind_matches(*kind, ch))
            || self
                .negated_kinds
                .iter()
                .any(|kind| !kind_matches(*kind, ch));
        hit != self.negated
    }
}

fn kind_matches(kind: Shorthand, ch: char) -> bool {
    match kind {
        Shorthand::Digit => ch.is_ascii_digit(),
        Shorthand::Word => ch == '_' || ch.is_ascii_alphanumeric(),
        Shorthand::Space => matches!(ch, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}'),
    }
}

// ----------------------------------------------------------------------
// String formats (hand-rolled; NO new dependencies)
// ----------------------------------------------------------------------

/// Validates a string against the v1 recognized formats: `date-time`,
/// `date`, `time`, `email`, `hostname`, `uri`, `uuid`.
///
/// Unknown formats are IGNORED (`Ok(())`) with the documented v1 note that
/// unrecognized names stay validation metadata only.
///
/// # Errors
///
/// [`Violation::Format`] naming the format and rejected value.
pub fn validate_format_string(value: &str, format: &str) -> Result<(), Violation> {
    let ok = match format {
        "date-time" => is_date_time(value),
        "date" => is_date(value),
        "time" => is_time(value),
        "email" => is_email(value),
        "hostname" => is_hostname(value),
        "uri" => is_uri(value),
        "uuid" => is_uuid(value),
        // Unknown formats stay metadata-only in v1.
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(Violation::Format {
            format: format.to_owned(),
            actual: value.to_owned(),
        })
    }
}

/// RFC 3339 full-date: `YYYY-MM-DD` with real calendar validity (leap years).
fn is_date(text: &str) -> bool {
    let Some((year_text, rest)) = text.split_once('-') else {
        return false;
    };
    let Some((month_text, day_text)) = rest.split_once('-') else {
        return false;
    };
    let (Some(year), Some(month), Some(day)) = (
        fixed_digits(year_text, 4),
        fixed_digits(month_text, 2),
        fixed_digits(day_text, 2),
    ) else {
        return false;
    };
    if !(1..=12).contains(&month) {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if leap => 29,
        _ => 28,
    };
    (1..=max_day).contains(&day)
}

fn fixed_digits(text: &str, width: usize) -> Option<u32> {
    if text.len() != width || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// RFC 3339 partial-time: `HH:MM:SS(.fraction)?` (leap second 60 allowed).
fn is_partial_time(text: &str) -> bool {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 3 {
        return false;
    }
    let (Some(hour), Some(minute)) = (fixed_digits(parts[0], 2), fixed_digits(parts[1], 2)) else {
        return false;
    };
    if hour > 23 || minute > 59 {
        return false;
    }
    let (second_text, fraction) = match parts[2].split_once('.') {
        Some((second_text, fraction)) => (second_text, Some(fraction)),
        None => (parts[2], None),
    };
    let Some(second) = fixed_digits(second_text, 2) else {
        return false;
    };
    if second > 60 {
        return false;
    }
    fraction
        .is_none_or(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

/// RFC 3339 time-offset: `Z` / `z` / `±HH:MM`.
fn is_time_offset(text: &str) -> bool {
    if text == "Z" || text == "z" {
        return true;
    }
    let Some(sign) = text.chars().next() else {
        return false;
    };
    if sign != '+' && sign != '-' {
        return false;
    }
    let Some((hour_text, minute_text)) = text[1..].split_once(':') else {
        return false;
    };
    let (Some(hour), Some(minute)) = (fixed_digits(hour_text, 2), fixed_digits(minute_text, 2))
    else {
        return false;
    };
    hour <= 23 && minute <= 59
}

/// RFC 3339 date-time: full-date, then `T`/`t`/space, then full-time.
/// (RFC 3339 §5.6 NOTE allows the space separator by mutual agreement.)
fn is_date_time(text: &str) -> bool {
    let Some(separator) = text.find(['T', 't', ' ']) else {
        return false;
    };
    let (date_part, rest) = text.split_at(separator);
    let time_part = &rest[1..];
    if date_part.is_empty() || time_part.is_empty() {
        return false;
    }
    // The offset marker is the LAST Z/z/+/- (fractions contain none of them).
    let Some(offset_index) = time_part.rfind(['Z', 'z', '+', '-']) else {
        return false;
    };
    if offset_index == 0 {
        return false;
    }
    is_date(date_part)
        && is_partial_time(&time_part[..offset_index])
        && is_time_offset(&time_part[offset_index..])
}

/// RFC 3339 full-time: partial-time plus offset.
fn is_time(text: &str) -> bool {
    let Some(offset_index) = text.rfind(['Z', 'z', '+', '-']) else {
        return false;
    };
    if offset_index == 0 {
        return false;
    }
    let (partial, offset) = text.split_at(offset_index);
    is_partial_time(partial) && is_time_offset(offset)
}

/// Pragmatic email (RFC 5322 domain subset): exactly one `@`, non-empty
/// local part without spaces or controls, and an RFC 1123 hostname domain.
fn is_email(text: &str) -> bool {
    let mut splits = text.split('@');
    let (Some(local), Some(domain)) = (splits.next(), splits.next()) else {
        return false;
    };
    if splits.next().is_some() || local.is_empty() || domain.is_empty() {
        return false;
    }
    if local
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return false;
    }
    is_hostname(domain)
}

/// RFC 1123 hostname: dot-separated labels of `[A-Za-z0-9-]`, each ≤ 63
/// bytes, no leading/trailing hyphen, total ≤ 253 bytes.
fn is_hostname(text: &str) -> bool {
    if text.is_empty() || text.len() > 253 || text.starts_with('.') || text.ends_with('.') {
        return false;
    }
    text.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

/// Pragmatic absolute URI (RFC 3986): scheme `[A-Za-z][A-Za-z0-9+.-]*`
/// followed by `:` and a non-empty remainder free of spaces, controls, and
/// a few wire-hostile characters (percent escapes allowed anywhere).
fn is_uri(text: &str) -> bool {
    let Some(separator) = text.find(':') else {
        return false;
    };
    let scheme = &text[..separator];
    let mut scheme_chars = scheme.chars();
    match scheme_chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    if !scheme_chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-')) {
        return false;
    }
    let rest = &text[separator + 1..];
    !rest.is_empty()
        && !rest
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, ' ' | '<' | '>' | '"'))
}

/// UUID: canonical 8-4-4-4-12 hex groups separated by hyphens, any case;
/// braced/urn forms stay outside v1.
fn is_uuid(text: &str) -> bool {
    const WIDTHS: [usize; 5] = [8, 4, 4, 4, 12];
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == WIDTHS.len()
        && groups.iter().zip(WIDTHS).all(|(group, width)| {
            group.len() == width && group.chars().all(|ch| ch.is_ascii_hexdigit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Lengths, objects, arrays, numbers
    // ------------------------------------------------------------------

    #[test]
    fn string_lengths_and_patterns_report_expected_versus_actual() {
        let constraints = StringConstraints {
            pattern: Some("^[a-z]+$"),
            min_length: Some(3),
            max_length: Some(5),
        };
        assert_eq!(validate_string("abc", &constraints), Ok(()));
        assert_eq!(
            validate_string("ab", &constraints),
            Err(Violation::MinLength {
                expected: 3,
                actual: 2
            })
        );
        assert_eq!(
            validate_string("abcdef", &constraints),
            Err(Violation::MaxLength {
                expected: 5,
                actual: 6
            })
        );
        assert_eq!(
            validate_string("AB1", &constraints),
            Err(Violation::Pattern {
                pattern: "^[a-z]+$".to_owned(),
                actual: "AB1".to_owned()
            })
        );
    }

    #[test]
    fn number_bounds_exclusivity_and_multiple_of_are_enforced() {
        assert_eq!(validate_number(5.0, Some((5.0, false)), None, None), Ok(()));
        assert_eq!(
            validate_number(5.0, Some((5.0, true)), None, None),
            Err(Violation::Minimum {
                expected: 5.0,
                exclusive: true,
                actual: 5.0
            })
        );
        assert_eq!(
            validate_number(100.0, None, Some((100.0, true)), None),
            Err(Violation::Maximum {
                expected: 100.0,
                exclusive: true,
                actual: 100.0
            })
        );
        assert_eq!(
            validate_number(3.2, None, None, Some(0.5)),
            Err(Violation::MultipleOf { expected: 0.5 })
        );
        // Decimal divisor: 0.3 / 0.1 is exactly 3 only within tolerance.
        assert_eq!(validate_number(0.3, None, None, Some(0.1)), Ok(()));
        assert_eq!(validate_number(-4.0, None, None, Some(2.0)), Ok(()));
    }

    #[test]
    fn array_and_object_cardinality_checks() {
        let constraints = ArrayConstraints {
            min_items: Some(2),
            max_items: Some(3),
        };
        assert_eq!(validate_array_len(2, &constraints), Ok(()));
        assert_eq!(
            validate_array_len(1, &constraints),
            Err(Violation::MinItems {
                expected: 2,
                actual: 1
            })
        );
        assert_eq!(
            validate_array_len(4, &constraints),
            Err(Violation::MaxItems {
                expected: 3,
                actual: 4
            })
        );
        assert_eq!(validate_object_props(2, Some(2), Some(2)), Ok(()));
        assert_eq!(
            validate_object_props(1, Some(2), None),
            Err(Violation::MinProperties {
                expected: 2,
                actual: 1
            })
        );
        assert_eq!(
            validate_object_props(3, None, Some(2)),
            Err(Violation::MaxProperties {
                expected: 2,
                actual: 3
            })
        );
        assert_eq!(validate_contains_count(1, Some(1), Some(2)), Ok(()));
        assert_eq!(
            validate_contains_count(0, Some(1), None),
            Err(Violation::ContainsMin {
                expected: 1,
                actual: 0
            })
        );
        assert_eq!(
            validate_contains_count(3, None, Some(2)),
            Err(Violation::ContainsMax {
                expected: 2,
                actual: 3
            })
        );
    }

    #[test]
    fn uniqueness_helpers_cover_strings_numbers_and_negative_zero() {
        assert_eq!(require_unique_strings(["a", "b"]), Ok(()));
        assert_eq!(require_unique_strings(vec![String::from("x")]), Ok(()));
        assert_eq!(
            require_unique_strings(["a", "a"]),
            Err(Violation::UniqueItems)
        );
        assert_eq!(require_unique_numbers([1.0, -0.0]), Ok(()));
        assert_eq!(
            require_unique_numbers([0.0, -0.0]),
            Err(Violation::UniqueItems)
        );
        assert_eq!(
            require_unique_numbers([1.5, 1.5]),
            Err(Violation::UniqueItems)
        );
    }

    // ------------------------------------------------------------------
    // Pattern subset coverage
    // ------------------------------------------------------------------

    fn decision(pattern: &str, input: &str) -> PatternDecision {
        evaluate_pattern(pattern, input)
    }

    #[test]
    fn literals_dots_anchors_and_shorthands_match() {
        assert_eq!(decision("abc", "abc"), PatternDecision::Match);
        assert_eq!(decision("abc", "xabcx"), PatternDecision::Match);
        assert_eq!(decision("^abc$", "abc"), PatternDecision::Match);
        assert_eq!(decision("^abc$", "xabc"), PatternDecision::NoMatch);
        assert_eq!(decision("a.c", "abc"), PatternDecision::Match);
        assert_eq!(decision("a.c", "a\nc"), PatternDecision::NoMatch);
        assert_eq!(decision(r"\d+", "12345"), PatternDecision::Match);
        assert_eq!(decision(r"^\D+$", "abc!"), PatternDecision::Match);
        assert_eq!(decision(r"^\w+$", "a_B9"), PatternDecision::Match);
        assert_eq!(decision(r"^\s$", "\t"), PatternDecision::Match);
        assert_eq!(decision(r"^\S\S$", "a "), PatternDecision::NoMatch);
    }

    #[test]
    fn classes_ranges_negation_and_escaped_members_match() {
        assert_eq!(decision("^[a-c]+$", "abcabc"), PatternDecision::Match);
        assert_eq!(decision("^[a-c]+$", "d"), PatternDecision::NoMatch);
        assert_eq!(decision("^[^a-c]+$", "xyz"), PatternDecision::Match);
        assert_eq!(decision("^[^a-c]+$", "b"), PatternDecision::NoMatch);
        assert_eq!(decision(r"^[\d]+$", "42"), PatternDecision::Match);
        assert_eq!(decision(r"^[\d\s]+$", "4 2"), PatternDecision::Match);
        assert_eq!(decision(r"^[\D]+$", "ab!"), PatternDecision::Match);
        assert_eq!(decision("[]a]+", "]a["), PatternDecision::Match);
        assert_eq!(decision("[-c]", "-"), PatternDecision::Match);
        assert_eq!(decision("[\\]]", "]"), PatternDecision::Match);
        assert_eq!(decision("[a\\-c]", "-"), PatternDecision::Match);
        assert_eq!(decision("[\\x41]", "A"), PatternDecision::Match);
        assert_eq!(decision("\\u00e9", "\u{e9}"), PatternDecision::Match);
        // A '{' that is not a quantifier stays literal.
        assert_eq!(decision("a{", "a{"), PatternDecision::Match);
        assert_eq!(decision("a{", "a"), PatternDecision::NoMatch);
    }

    #[test]
    fn alternation_groups_and_quantifiers_greedy_and_lazy() {
        assert_eq!(decision("^(cat|dog)$", "cat"), PatternDecision::Match);
        assert_eq!(decision("^(cat|dog)$", "cow"), PatternDecision::NoMatch);
        assert_eq!(decision("^(a|b)+$", "abba"), PatternDecision::Match);
        assert_eq!(decision("^(?:ab)+$", "abab"), PatternDecision::Match);
        assert_eq!(decision("^a*$", ""), PatternDecision::Match);
        assert_eq!(decision("^a+$", "aaa"), PatternDecision::Match);
        assert_eq!(decision("^a?a?$", "aa"), PatternDecision::Match);
        assert_eq!(decision("^a{3}$", "aaa"), PatternDecision::Match);
        assert_eq!(decision("^a{3}$", "aa"), PatternDecision::NoMatch);
        assert_eq!(decision("^a{2,}$", "aaaa"), PatternDecision::Match);
        assert_eq!(decision("^a{2,4}$", "aaa"), PatternDecision::Match);
        assert_eq!(decision("^a{2,4}?$", "aaa"), PatternDecision::Match);
        assert_eq!(decision("^a{4,2}$", "aaaa"), PatternDecision::Unsupported);
        // Empty-body repetitions terminate via the progress guard.
        assert_eq!(decision("^(a*)*$", "aaa"), PatternDecision::Match);
        assert_eq!(decision("^(|a)*$", "aaa"), PatternDecision::Match);
    }

    #[test]
    fn nested_quantifiers_within_budget_still_decide() {
        assert_eq!(decision("^(ab)+$", "ababab"), PatternDecision::Match);
        assert_eq!(
            decision(r"^([a-z]{2}\d){2}$", "ab1cd2"),
            PatternDecision::Match
        );
        assert_eq!(
            decision(r"^([a-z]{2}\d){2}$", "ab12"),
            PatternDecision::NoMatch
        );
        // Lazy vs greedy differ on capture-free matching only in preference
        // order, so both decide identically here.
        assert_eq!(decision("^a+?$", "aaa"), PatternDecision::Match);
    }

    #[test]
    fn unsupported_constructs_report_unsupported() {
        assert_eq!(decision("(?=x)a", "xa"), PatternDecision::Unsupported);
        assert_eq!(decision("(?!x)a", "ya"), PatternDecision::Unsupported);
        assert_eq!(decision(r"(a)\1", "aa"), PatternDecision::Unsupported);
        assert_eq!(decision("(?<name>a)", "a"), PatternDecision::Unsupported);
        assert_eq!(decision("(?i)a", "A"), PatternDecision::Unsupported);
        assert_eq!(decision(r"a\bb", "ab"), PatternDecision::Unsupported);
        assert_eq!(decision("[z-a]", "a"), PatternDecision::Unsupported);
        assert_eq!(
            decision("(unclosed", "unclosed"),
            PatternDecision::Unsupported
        );
        assert_eq!(decision("*greedy", "x"), PatternDecision::Unsupported);
        assert_eq!(decision(")", ")"), PatternDecision::Unsupported);
    }

    #[test]
    fn pathological_backtracking_aborts_quickly_instead_of_hanging() {
        // Catastrophic case `(a+)+$` against many 'a's and one 'b'.
        let input = "a".repeat(48) + "b";
        let started = std::time::Instant::now();
        let outcome = decision("^(a+)+$", &input);
        let elapsed = started.elapsed();
        assert!(
            matches!(
                outcome,
                PatternDecision::Unsupported | PatternDecision::NoMatch
            ),
            "pathological pattern must abort or fail fast, got {outcome:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "matcher hung for {elapsed:?}; step budget failed"
        );
        // Sanity: the same pattern still matches ordinary inputs.
        assert_eq!(decision("^(a+)+$", "aaa"), PatternDecision::Match);
        // And the lenient primitive skips the undecidable attempt entirely.
        let constraints = StringConstraints {
            pattern: Some("^(a+)+$"),
            min_length: None,
            max_length: None,
        };
        assert_eq!(validate_string(&input, &constraints), Ok(()));
    }

    #[test]
    fn validate_string_enforces_lengths_even_when_patterns_are_skipped() {
        // Lookahead patterns are skipped; other constraints still enforce.
        let constraints = StringConstraints {
            pattern: Some("^(?=.*x)y*"),
            min_length: Some(3),
            max_length: None,
        };
        assert_eq!(
            validate_string("yy", &constraints),
            Err(Violation::MinLength {
                expected: 3,
                actual: 2
            })
        );
        let passing = StringConstraints {
            pattern: Some("(?=never-decided)whatever"),
            min_length: None,
            max_length: None,
        };
        assert_eq!(validate_string("", &passing), Ok(()));
    }

    // ------------------------------------------------------------------
    // Formats
    // ------------------------------------------------------------------

    #[test]
    fn date_time_samples_accept_and_reject() {
        for good in [
            "2026-08-24T12:34:56Z",
            "2026-08-24t12:34:56.123+02:00",
            "2024-02-29T00:00:00Z",
            "2026-08-24 12:34:56Z",
        ] {
            assert_eq!(validate_format_string(good, "date-time"), Ok(()), "{good}");
        }
        for bad in [
            "not-a-date",
            "2026-13-01T00:00:00Z",
            "2026-08-24T24:00:00Z",
            "2026-08-24T12:34:56",
            "2023-02-29T00:00:00Z",
        ] {
            assert!(
                matches!(
                    validate_format_string(bad, "date-time"),
                    Err(Violation::Format { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn remaining_formats_accept_and_reject_samples() {
        for good in [
            ("2026-08-24", "date"),
            ("1999-12-31", "date"),
            ("12:34:56Z", "time"),
            ("23:59:60.5-05:30", "time"),
            ("user@example.com", "email"),
            ("user.name+tag@sub.example.co", "email"),
            ("example.com", "hostname"),
            ("a-b.example.org", "hostname"),
            ("https://example.com/x?y=1#z", "uri"),
            ("urn:isbn:0451450523", "uri"),
            ("123e4567-e89b-12d3-a456-426614174000", "uuid"),
            ("123E4567-E89B-12D3-A456-426614174000", "uuid"),
        ] {
            assert_eq!(validate_format_string(good.0, good.1), Ok(()), "{good:?}");
        }
        for bad in [
            ("2026-02-30", "date"),
            ("12:34:56", "time"),
            ("@example.com", "email"),
            ("two@@example.com", "email"),
            ("-example.com", "hostname"),
            ("example-.com", "hostname"),
            ("//example.com", "uri"),
            ("ht tp://x", "uri"),
            ("123e456-e89b-12d3-a456-426614174000", "uuid"),
            ("123e4567e89b12d3a456426614174000", "uuid"),
        ] {
            assert!(
                matches!(
                    validate_format_string(bad.0, bad.1),
                    Err(Violation::Format { .. })
                ),
                "{bad:?}"
            );
        }
        // Unknown formats stay ignored (metadata-only in v1).
        assert_eq!(validate_format_string("anything", "int32"), Ok(()));
    }

    // ------------------------------------------------------------------
    // Field annotation (companion §9 rejection detail)
    // ------------------------------------------------------------------

    #[test]
    fn at_field_wraps_and_composes_paths() {
        let violation = Violation::MinLength {
            expected: 3,
            actual: 2,
        };
        let annotated = violation.clone().at_field("code");
        assert_eq!(
            annotated,
            Violation::Field {
                field: String::from("code"),
                source: Box::new(violation),
            }
        );
        let nested = annotated.at_field("ticket").at_field("body");
        assert_eq!(
            nested.to_string(),
            "field `body`: field `ticket`: field `code`: length 2 is below minLength 3"
        );
        // The innermost constraint stays observable for strict modes.
        assert!(matches!(
            nested.innermost(),
            Violation::MinLength {
                expected: 3,
                actual: 2
            }
        ));
    }
}
