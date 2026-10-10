//! What a column edge means, not just that it exists.
//!
//! This is the whole point of the project: Snowflake's own lineage says
//! `a` feeds `b` and stops there. A role says whether `b` is `a` unchanged,
//! `a` renamed, `a` cast, or `a` buried inside a window function.
//!
//! Classification reads the expression text the engine attaches to a derived
//! column. It is deliberately conservative: anything it cannot place lands in
//! `Transform`, which claims nothing beyond "an expression was involved".

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    Passthrough,
    Rename,
    Cast,
    Aggregate,
    Window,
    Transform,
    /// A hash of its inputs and nothing else: a surrogate key, a hashdiff. The
    /// value is not the input in any readable sense, yet it depends on every
    /// one of them exactly, which neither `transform` nor `aggregate` says.
    Hash,
    // The three below are indirect: the column decided which rows exist rather
    // than what any one value is, so it reaches every output column of the
    // model and none of them in particular. Named apart from the six above so
    // that neither can ever be read as the other.
    JoinKey,
    DedupKey,
    Filter,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Passthrough => "passthrough",
            Role::Rename => "rename",
            Role::Cast => "cast",
            Role::Aggregate => "aggregate",
            Role::Window => "window",
            Role::Transform => "transform",
            Role::Hash => "hash",
            Role::JoinKey => "join_key",
            Role::DedupKey => "dedup_key",
            Role::Filter => "filter",
        }
    }

    /// How much a role claims, so that a column carried through several
    /// expressions is described by the loudest of them rather than by the last.
    ///
    /// A CTE that computes `sum(x) as total` followed by `total * 1.2 as gross`
    /// is an aggregate that was then scaled. Reporting only the scaling is true
    /// but useless: `transform` is what every unrecognised expression already
    /// says, and the aggregate is the fact a reader came for.
    ///
    /// An indirect role sits at zero and never competes with one of these. It
    /// describes a different relationship, not a stronger one, and the two
    /// never meet on a path. Two indirect roles do meet, on a column read for
    /// two reasons, and `indirect_precedence` is what orders those.
    pub fn rank(self) -> u8 {
        match self {
            Role::JoinKey | Role::DedupKey | Role::Filter => 0,
            Role::Passthrough => 0,
            Role::Rename => 1,
            Role::Cast => 2,
            Role::Transform => 3,
            // Above transform, which it is a named case of; below aggregate,
            // which a hash over an aggregate still is.
            Role::Hash => 4,
            Role::Aggregate => 5,
            Role::Window => 6,
        }
    }

    /// True when the column feeds no single output column, only the choice of
    /// rows. `classify` never returns one of these: they come from a clause,
    /// not from an expression behind an output column.
    pub fn is_indirect(self) -> bool {
        matches!(self, Role::JoinKey | Role::DedupKey | Role::Filter)
    }

    /// Which indirect role an edge carries when one column plays several, as
    /// in `where valid_from <= current_date qualify valid_from = max(valid_from)
    /// over ()`. The cache holds one edge per column pair (0013).
    ///
    /// A filter says only that the column was tested, and the other two imply
    /// as much, so it is the floor. A dedup key tops a join key: `QUALIFY` runs
    /// last, on rows already joined and filtered, and it fixes the grain of the
    /// model, which is the most specific thing to say about which rows exist.
    ///
    /// Apart from `rank` so that no indirect role can ever be compared with a
    /// value role: `None` for the six of those.
    pub fn indirect_precedence(self) -> Option<u8> {
        match self {
            Role::Filter => Some(0),
            Role::JoinKey => Some(1),
            Role::DedupKey => Some(2),
            Role::Passthrough
            | Role::Rename
            | Role::Cast
            | Role::Aggregate
            | Role::Window
            | Role::Transform
            | Role::Hash => None,
        }
    }
}

const AGGREGATES: &[&str] = &[
    "sum", "count", "avg", "min", "max", "median", "mode", "stddev", "stddev_pop", "stddev_samp",
    "variance", "var_pop", "var_samp", "listagg", "array_agg", "object_agg", "any_value",
    "approx_count_distinct", "percentile_cont", "percentile_disc", "corr", "covar_pop", "covar_samp",
    "bitand_agg", "bitor_agg", "bitxor_agg", "hash_agg", "grouping",
];

/// A window is any expression carrying an OVER clause. Checked before the
/// aggregate test, because `sum(x) over (...)` is a window function, not an
/// aggregate, and calling it an aggregate would be the more misleading answer.
fn has_over(expr: &str) -> bool {
    let low = expr.to_lowercase();
    let bytes = low.as_bytes();
    let mut from = 0;
    while let Some(i) = low[from..].find("over") {
        let at = from + i;
        let before_ok = at == 0 || !bytes[at - 1].is_ascii_alphanumeric() && bytes[at - 1] != b'_';
        let mut j = at + 4;
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        if before_ok && j < bytes.len() && bytes[j] == b'(' {
            return true;
        }
        from = at + 4;
    }
    false
}

/// Functions whose result is a hash of their input. `hash_agg` is an aggregate
/// and stays one.
const HASHES: &[&str] = &[
    "md5", "md5_binary", "md5_hex", "sha1", "sha1_binary", "sha1_hex", "sha2", "sha2_binary",
    "sha2_hex", "hash",
];

/// True when the whole expression is one call to a hash function, once the
/// casts round it are peeled: `cast(sha1_binary(concat_ws('||', a, b)) as
/// binary(20))`, `md5(...)::varchar`, `(md5(x))`. Not a hash with something
/// done to it, `coalesce(md5(x), '-1')` or `md5(a) = md5(b)`: those claim less.
fn is_hash(expr: &str) -> bool {
    let mut t = strip_parens(expr);
    loop {
        let bytes = t.as_bytes();
        let opener = ["cast(", "try_cast("]
            .into_iter()
            .find(|p| bytes.len() >= p.len() && bytes[..p.len()].eq_ignore_ascii_case(p.as_bytes()));
        if let Some(open) = opener {
            if closing(t, open.len() - 1) == Some(t.len() - 1) {
                match cast_subject(&t[open.len()..t.len() - 1]) {
                    Some(subject) => {
                        t = strip_parens(subject);
                        continue;
                    }
                    None => return false,
                }
            }
        }
        if let Some(i) = last_top_level_cast(t) {
            if is_type(&t[i + 2..]) {
                t = strip_parens(&t[..i]);
                continue;
            }
        }
        break;
    }
    match (leading_call(t), t.find('(')) {
        (Some(call), Some(open)) => {
            HASHES.contains(&call.as_str()) && closing(t, open) == Some(t.len() - 1)
        }
        _ => false,
    }
}

/// True when the expression calls an aggregate anywhere, not only at its head:
/// `coalesce(sum(amount), 0)` is an aggregate with a default, and reading the
/// leading call alone called it a transform. A whole word from `AGGREGATES`
/// followed by a parenthesis, outside quotes: `my_sum(` and `max_amount` are
/// not calls to one.
fn calls_aggregate(expr: &str) -> bool {
    let bytes = expr.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            q @ (b'\'' | b'"') => {
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                i += 1;
            }
            b if ident(b) => {
                let start = i;
                while i < bytes.len() && ident(bytes[i]) {
                    i += 1;
                }
                let word = expr[start..i].to_ascii_lowercase();
                let qualified = start > 0 && bytes[start - 1] == b'.';
                let mut j = i;
                while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                    j += 1;
                }
                if !qualified && bytes.get(j) == Some(&b'(') && AGGREGATES.contains(&word.as_str()) {
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// The function a call expression starts with, lower case. Shared with the
/// engine, which uses it to spot a column the engine named after its own
/// function rather than after an alias.
pub(crate) fn leading_call(expr: &str) -> Option<String> {
    let t = expr.trim();
    let open = t.find('(')?;
    let name: String = t[..open].trim().to_lowercase();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(name)
}

/// A column and nothing else: `amount`, `o.amount`, `"Amount"`.
fn is_bare_name(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '"' || c == '$')
}

/// Where the parenthesis opened at `open` closes, reading past quoted text: a
/// literal or a quoted name can spell a parenthesis without being one.
fn closing(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'\'' | b'"' => quote = Some(b),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
        }
    }
    None
}

/// The text inside every pair of parentheses that wraps all of it, so that
/// `((amount))` reads as `amount`. Only a pair that opens first and closes
/// last: `(a) + (b)` is left alone.
fn strip_parens(s: &str) -> &str {
    let mut t = s.trim();
    while t.starts_with('(') && closing(t, 0) == Some(t.len() - 1) {
        t = t[1..t.len() - 1].trim();
    }
    t
}

/// Where the last `::` outside parentheses and quotes starts: `a::int::varchar`
/// is a cast of `a::int`, and a `::` inside a call belongs to the call.
fn last_top_level_cast(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let (mut depth, mut quote, mut last) = (0i32, None::<u8>, None);
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'\'' | b'"' => quote = Some(b),
                b'(' => depth += 1,
                b')' => depth -= 1,
                b':' if depth == 0 && bytes.get(i + 1) == Some(&b':') => {
                    last = Some(i);
                    i += 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    last
}

/// Types spelled in more than one word, as the parser prints them.
const MULTI_WORD_TYPES: &[&str] = &[
    "DOUBLE PRECISION",
    "CHARACTER VARYING",
    "CHAR VARYING",
    "TIMESTAMP WITH TIME ZONE",
    "TIMESTAMP WITHOUT TIME ZONE",
    "TIMESTAMP WITH LOCAL TIME ZONE",
];

/// A type and nothing else: one word, or one of `MULTI_WORD_TYPES`, with an
/// optional `(precision)` or `(precision, scale)`. A closed grammar on purpose:
/// `int is null` and `int + b` are not types, and a type missing from the list
/// reads as no type, which leaves the edge a transform, the safe side.
fn is_type(s: &str) -> bool {
    let t = s.trim();
    let (name, args) = match t.find('(') {
        Some(open) if closing(t, open) == Some(t.len() - 1) => (&t[..open], Some(&t[open + 1..t.len() - 1])),
        Some(_) => return false,
        None => (t, None),
    };
    let args_ok = args.is_none_or(|a| {
        let parts: Vec<&str> = a.split(',').map(str::trim).collect();
        parts.len() <= 2 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    });
    let words: Vec<&str> = name.split_whitespace().collect();
    let one_word = words.len() == 1 && words[0].chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let listed = MULTI_WORD_TYPES.iter().any(|m| m.eq_ignore_ascii_case(&words.join(" ")));
    args_ok && (one_word || listed)
}

/// One value: a column, or a cast of one, whatever parentheses wrap it.
fn is_single_value(s: &str) -> bool {
    let t = strip_parens(s);
    is_bare_name(t) || is_pure_cast(t)
}

/// The thing being cast, when the text is `<something> as <type>` at depth zero.
///
/// Depth matters: `md5(concat(a, b)) as binary(20)` must yield the call, not the
/// first `as` it happens to contain. So do quotes: `'a as b'` is a literal.
fn cast_subject(inner: &str) -> Option<&str> {
    let bytes = inner.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        if let Some(q) = quote {
            if bytes[i] == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'\'' | b'"' => quote = Some(bytes[i]),
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'a' | b'A' if depth == 0 => {
                let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
                // `as` is ASCII, so comparing bytes is exact. Lowercasing the
                // whole string first would move the byte offsets it is then
                // read with: three code points grow a byte when lowercased,
                // after which `i` lands inside a character and slicing panics.
                let is_as = matches!(bytes.get(i + 1), Some(b's') | Some(b'S'));
                if before_ok && is_as {
                    let after = i + 2;
                    if after < bytes.len() && (bytes[after] as char).is_whitespace() {
                        return Some(&inner[..i]);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// True when the expression is nothing but a cast of one value, in either of
/// Snowflake's two spellings, whatever parentheses wrap it.
fn is_pure_cast(expr: &str) -> bool {
    let t = strip_parens(expr);
    let bytes = t.as_bytes();
    let opener = ["cast(", "try_cast("]
        .into_iter()
        .find(|p| bytes.len() >= p.len() && bytes[..p.len()].eq_ignore_ascii_case(p.as_bytes()));
    if let Some(open) = opener {
        // Only when the call is the whole text: `cast(a as int)::varchar` is a
        // cast of a cast, read below, and `cast(a as int) + b` is no cast.
        if closing(t, open.len() - 1) == Some(t.len() - 1) {
            // A cast is only a cast when it casts one value. AutomateDV wraps
            // its hashdiff in one, `cast(md5_binary(concat(a, b, c)) as
            // binary(20))`, and calling that a cast would claim the value came
            // through unchanged when it is a hash of thirty columns.
            let inner = &t[open.len()..t.len() - 1];
            return cast_subject(inner).is_some_and(is_single_value);
        }
    }
    // `x::type`, split at the last `::` outside parentheses, so that neither
    // `a + b::int` nor `x::int is null` is taken for one: the left must be one
    // value and the right a type and nothing more.
    match last_top_level_cast(t) {
        Some(i) => is_single_value(&t[..i]) && is_type(&t[i + 2..]),
        None => false,
    }
}

/// The role of an output column, given the expression behind it (empty when the
/// column was copied straight through) and the two names involved.
pub fn classify(expr: &str, source_column: &str, output_column: &str) -> Role {
    let e = expr.trim();
    if e.is_empty() {
        return if source_column.eq_ignore_ascii_case(output_column) {
            Role::Passthrough
        } else {
            Role::Rename
        };
    }
    if has_over(e) {
        return Role::Window;
    }
    if calls_aggregate(e) {
        return Role::Aggregate;
    }
    if is_hash(e) {
        return Role::Hash;
    }
    if let Some(call) = leading_call(e) {
        if call == "cast" || call == "try_cast" {
            return if is_pure_cast(e) { Role::Cast } else { Role::Transform };
        }
        return Role::Transform;
    }
    if is_pure_cast(e) {
        return Role::Cast;
    }
    Role::Transform
}

/// What an expression claims on its own, with no names to compare. Used to rank
/// two expressions met on one path; `classify` is what names an edge.
pub fn of_expression(expr: &str) -> Role {
    classify(expr, "", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cast_in_parentheses_or_of_a_cast_is_still_a_cast() {
        assert_eq!(classify("(amount)::decimal(18,4)", "amount", "amt"), Role::Cast);
        assert_eq!(classify("CAST((o.amount) AS VARCHAR)", "amount", "amt"), Role::Cast);
        assert_eq!(classify("cast(try_cast(code as int) as varchar)", "code", "code"), Role::Cast);
        assert_eq!(classify("cast(code as int)::varchar", "code", "code"), Role::Cast);
        assert_eq!(classify("CAST(\"X\" AS CHARACTER VARYING(1))", "x", "x"), Role::Cast);
        assert_eq!(classify("(cast(amount as varchar))", "amount", "amt"), Role::Cast);
    }

    #[test]
    fn a_cast_that_is_only_an_operand_is_not_a_cast() {
        assert_eq!(classify("a::int + b", "b", "total"), Role::Transform);
        assert_eq!(classify("cast(a as int) + cast(b as int)", "b", "total"), Role::Transform);
        assert_eq!(classify("(d)::date between lo and hi", "hi", "is_open"), Role::Transform);
        assert_eq!(classify("x::int is null", "x", "missing"), Role::Transform);
        // A quote spells what it likes.
        assert_eq!(classify("'a::int'", "a", "a"), Role::Transform);
    }

    #[test]
    fn a_conversion_function_is_left_a_transform() {
        // Deliberately: `to_char(d, 'YYYY-MM')` truncates and `to_timestamp(x,
        // 3)` changes the unit of an epoch, and telling those from a plain
        // conversion is a list of formats this does not keep.
        assert_eq!(classify("try_to_decimal(amount, 18, 4)", "amount", "amt"), Role::Transform);
        assert_eq!(classify("to_varchar(code)", "code", "code"), Role::Transform);
    }

    #[test]
    fn a_cast_wrapped_round_an_expression_is_not_a_cast() {
        // The honest cases stay cast.
        assert_eq!(classify("cast(a as varchar)", "a", "a"), Role::Cast);
        assert_eq!(classify("a::varchar", "a", "a"), Role::Cast);
        // AutomateDV's hashdiff: a cast on the outside, a hash of many columns
        // underneath. Calling it a cast would claim the value is unchanged.
        assert_eq!(
            classify("cast(md5_binary(concat_ws('||', a, b)) as binary(20))", "a", "h"),
            Role::Hash
        );
        assert_eq!(classify("(a || b)::varchar", "a", "c"), Role::Transform);
        assert_eq!(classify("try_cast(upper(a) as varchar)", "a", "a"), Role::Transform);
    }

    #[test]
    fn an_indirect_role_is_never_something_classify_can_return() {
        for r in [Role::JoinKey, Role::DedupKey, Role::Filter] {
            assert!(r.is_indirect());
        }
        for e in ["", "sum(x)", "a || b", "x::int", "row_number() over (order by a)"] {
            assert!(!classify(e, "a", "b").is_indirect(), "{e}");
        }
    }

    #[test]
    fn indirect_roles_are_ordered_among_themselves_and_against_nothing_else() {
        let p = Role::indirect_precedence;
        assert!(p(Role::DedupKey) > p(Role::JoinKey));
        assert!(p(Role::JoinKey) > p(Role::Filter));
        let values = [Role::Passthrough, Role::Rename, Role::Cast];
        for r in values.into_iter().chain([Role::Aggregate, Role::Window, Role::Transform]) {
            assert_eq!(p(r), None, "{}", r.as_str());
        }
        // 0009 still holds: on a path they claim nothing over a value role.
        for r in [Role::JoinKey, Role::DedupKey, Role::Filter] {
            assert_eq!(r.rank(), 0, "{}", r.as_str());
        }
    }

    #[test]
    fn a_column_copied_through_keeps_its_name() {
        assert_eq!(classify("", "id", "id"), Role::Passthrough);
        assert_eq!(classify("", "ID", "id"), Role::Passthrough, "case must not matter");
        assert_eq!(classify("", "id", "order_id"), Role::Rename);
    }

    #[test]
    fn casts_are_told_apart_from_other_expressions() {
        assert_eq!(classify("amount::number(18,2)", "amount", "amt"), Role::Cast);
        assert_eq!(classify("cast(amount as varchar)", "amount", "amt"), Role::Cast);
        assert_eq!(classify("a + b::int", "a", "x"), Role::Transform);
    }

    #[test]
    fn the_loudest_expression_on_a_path_is_the_one_that_wins() {
        let r = |e: &str| of_expression(e).rank();
        assert!(r("sum(x)") > r("total * 1.2"), "an aggregate outranks the arithmetic round it");
        assert!(r("row_number() over (order by a)") > r("sum(x)"));
        assert!(r("a || b") > r("a::int"));
        assert!(r("a::int") > r(""));
        assert_eq!(of_expression(""), Role::Passthrough);
    }

    #[test]
    fn an_aggregate_below_the_head_of_an_expression_is_an_aggregate() {
        for e in [
            "coalesce(sum(amount), 0)",
            "(sum(debit) - sum(credit))",
            "iff(count(distinct c) = 1, max(c), null)",
            "CAST(SUM(x) AS DECIMAL(18,2))",
            "count(*)",
        ] {
            assert_eq!(classify(e, "x", "y"), Role::Aggregate, "{e}");
        }
        for e in ["my_sum(a)", "summary(a)", "coalesce(max_amount, 0)", "'sum(' || a", "t.max(a)"] {
            assert_eq!(classify(e, "a", "y"), Role::Transform, "{e}");
        }
        // A window still wins, whatever it wraps or is wrapped in.
        assert_eq!(classify("coalesce(sum(x) over (partition by y), 0)", "x", "y"), Role::Window);
    }

    #[test]
    fn a_hash_of_its_inputs_is_a_hash_whatever_casts_wrap_it() {
        for e in [
            "CAST(SHA1_BINARY(NULLIF(CONCAT_WS('||', IFNULL(NULLIF(UPPER(TRIM(CAST(a AS VARCHAR))), ''), '^^')), '^^')) AS BINARY(20))",
            "md5(cast(coalesce(cast(a as TEXT), '_null_') || '-' || coalesce(cast(b as TEXT), '_null_') as TEXT))",
            "try_cast(md5_binary(concat(a, b)) as binary(16))",
            "sha2(a, 256)::varchar",
            "(md5(a))",
        ] {
            assert_eq!(classify(e, "a", "hk"), Role::Hash, "{e}");
        }
        assert_eq!(classify("coalesce(md5(a), '-1')", "a", "hk"), Role::Transform);
        assert_eq!(classify("md5(a) = md5(b)", "a", "same"), Role::Transform);
        assert_eq!(classify("upper(md5(a))", "a", "hk"), Role::Transform);
        assert_eq!(classify("hash_agg(a)", "a", "h"), Role::Aggregate);
        assert_eq!(classify("cast(hk as binary(20))", "hk", "hk"), Role::Cast);
        assert_eq!(classify("row_number() over (partition by md5(a) order by b)", "a", "rn"), Role::Window);
        let r = |e: &str| of_expression(e).rank();
        assert!(r("md5(a)") > r("a || b") && r("md5(a)") < r("sum(a)"));
    }

    #[test]
    fn a_window_is_not_an_aggregate_even_when_it_wraps_one() {
        assert_eq!(classify("sum(x) over (partition by y)", "x", "running"), Role::Window);
        assert_eq!(classify("row_number() over (order by ts)", "ts", "rn"), Role::Window);
        assert_eq!(classify("sum(x)", "x", "total"), Role::Aggregate);
    }

    #[test]
    fn over_inside_an_identifier_is_not_a_window() {
        assert_eq!(classify("coalesce(overdraft, 0)", "overdraft", "od"), Role::Transform);
        assert_eq!(classify("handover(x)", "x", "y"), Role::Transform);
    }

    #[test]
    fn anything_unrecognised_claims_nothing_more_than_transform() {
        assert_eq!(classify("case when a then 1 else 2 end", "a", "b"), Role::Transform);
        assert_eq!(classify("a || b", "a", "c"), Role::Transform);
    }

    #[test]
    fn a_character_that_grows_when_lowercased_still_reads_as_a_cast() {
        // U+0130, U+023A and U+023E take one more byte in lower case. Reading a
        // byte offset taken from the original against the lowercased text put
        // the offset inside a character, which panicked on the slice and,
        // short of that, lost the `as` and demoted a cast to a transform.
        // A Turkish column name is enough to reach it.
        for subject in ["\u{130}SIM", "\u{130}a", "\u{23A}a", "\u{23E}a", "\u{130}\u{130}a"] {
            let expr = format!("cast({subject} as varchar)");
            assert_eq!(classify(&expr, "a", "b"), Role::Cast, "{expr}");
        }
        // The guarantee the depth walk exists for survives the change: a hash
        // wrapped in a cast is never taken for the cast.
        assert_eq!(
            classify("cast(md5_binary(concat_ws('||', \u{130}a, b)) as binary(20))", "a", "h"),
            Role::Hash
        );
    }
}
