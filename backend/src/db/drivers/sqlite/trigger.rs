//! Reads the head of a SQLite `CREATE TRIGGER` statement.
//!
//! SQLite keeps no column for the time or the event of a trigger. The
//! catalog keeps the text of the statement alone, so the driver reads these
//! facts from the words before the body of the trigger. The column list
//! of an `UPDATE OF` clause also comes from these words.

use crate::db::{TriggerEvent, TriggerTiming};

/// The facts of a trigger that its head gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerHead {
    pub timing: TriggerTiming,
    /// The change that fires the trigger. A head that this reader cannot
    /// read gives `None`.
    pub event: Option<TriggerEvent>,
    /// The columns of an `UPDATE OF` clause, in the order of the clause.
    pub update_columns: Vec<String>,
    /// The schema that the `ON` clause names before the table, if any.
    pub target_schema: Option<String>,
}

/// One word of the head. A quoted name is never a keyword.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    text: String,
    quoted: bool,
}

impl Token {
    fn is(&self, word: &str) -> bool {
        !self.quoted && self.text.eq_ignore_ascii_case(word)
    }
}

/// Reads a quoted region that starts at the opening mark. A doubled closing
/// mark stands for the mark itself. Returns the text inside the marks and
/// the place after the closing mark.
fn quoted(chars: &[char], at: usize, closing: char) -> (String, usize) {
    let mut text = String::new();
    let mut index = at + 1;
    while index < chars.len() {
        if chars[index] == closing {
            if closing != ']' && chars.get(index + 1) == Some(&closing) {
                text.push(closing);
                index += 2;
                continue;
            }
            return (text, index + 1);
        }
        text.push(chars[index]);
        index += 1;
    }
    (text, index)
}

/// Splits the text into words, quoted names and single marks. The comments
/// and the blanks go.
fn tokens(sql: &str) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let in_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current.is_whitespace() {
            index += 1;
        } else if current == '-' && chars.get(index + 1) == Some(&'-') {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
        } else if current == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            while index < chars.len()
                && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
            {
                index += 1;
            }
            index = (index + 2).min(chars.len());
        } else if matches!(current, '"' | '\'' | '`' | '[') {
            let closing = if current == '[' { ']' } else { current };
            let (text, next) = quoted(&chars, index, closing);
            out.push(Token { text, quoted: true });
            index = next;
        } else if in_word(current) {
            let start = index;
            while index < chars.len() && in_word(chars[index]) {
                index += 1;
            }
            out.push(Token {
                text: chars[start..index].iter().collect(),
                quoted: false,
            });
        } else {
            out.push(Token {
                text: current.to_string(),
                quoted: false,
            });
            index += 1;
        }
    }
    out
}

/// Reads the time, the event and the schema of the table of one trigger.
///
/// The grammar of the head is `CREATE [TEMP] TRIGGER [IF NOT EXISTS]
/// [schema.]name [BEFORE | AFTER | INSTEAD OF] {DELETE | INSERT | UPDATE
/// [OF columns]} ON [schema.]table`. A head without a time runs before the
/// change, as SQLite does. SQLite accepted the text when it made the
/// trigger, so a part that this reader cannot find gives its default.
pub fn trigger_head(sql: &str) -> TriggerHead {
    let tokens = tokens(sql);
    let word = |at: usize, wanted: &str| tokens.get(at).is_some_and(|token| token.is(wanted));
    let mut head = TriggerHead {
        timing: TriggerTiming::Before,
        event: None,
        update_columns: Vec::new(),
        target_schema: None,
    };

    let Some(mut at) = tokens.iter().position(|token| token.is("TRIGGER")) else {
        return head;
    };
    at += 1;
    if word(at, "IF") && word(at + 1, "NOT") && word(at + 2, "EXISTS") {
        at += 3;
    }
    // The name of the trigger, with its schema.
    at += if word(at + 1, ".") { 3 } else { 1 };

    if word(at, "BEFORE") {
        at += 1;
    } else if word(at, "AFTER") {
        head.timing = TriggerTiming::After;
        at += 1;
    } else if word(at, "INSTEAD") && word(at + 1, "OF") {
        head.timing = TriggerTiming::InsteadOf;
        at += 2;
    }

    head.event = [
        ("DELETE", TriggerEvent::Delete),
        ("INSERT", TriggerEvent::Insert),
        ("UPDATE", TriggerEvent::Update),
    ]
    .into_iter()
    .find(|(name, _)| word(at, name))
    .map(|(_, event)| event);

    // The column list of an update ends at the `ON` clause. A comma between
    // two names is a mark, and a quoted comma is a name.
    if head.event == Some(TriggerEvent::Update) && word(at + 1, "OF") {
        head.update_columns = tokens[at + 2..]
            .iter()
            .take_while(|token| !token.is("ON"))
            .filter(|token| !token.is(","))
            .map(|token| token.text.clone())
            .collect();
    }

    // The `ON` clause follows the event and the column list of an update.
    if let Some(on) = (at..tokens.len()).find(|&index| word(index, "ON")) {
        if word(on + 2, ".") {
            head.target_schema = tokens.get(on + 1).map(|token| token.text.clone());
        }
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(timing: TriggerTiming, event: TriggerEvent, schema: Option<&str>) -> TriggerHead {
        TriggerHead {
            timing,
            event: Some(event),
            update_columns: Vec::new(),
            target_schema: schema.map(str::to_string),
        }
    }

    #[test]
    fn a_head_without_a_time_runs_before_the_change() {
        assert_eq!(
            trigger_head("CREATE TRIGGER t INSERT ON orders BEGIN SELECT 1; END"),
            head(TriggerTiming::Before, TriggerEvent::Insert, None)
        );
    }

    #[test]
    fn each_time_and_each_event_is_read() {
        assert_eq!(
            trigger_head("create trigger t before delete on orders begin select 1; end"),
            head(TriggerTiming::Before, TriggerEvent::Delete, None)
        );
        assert_eq!(
            trigger_head("CREATE TRIGGER t AFTER INSERT ON orders BEGIN SELECT 1; END"),
            head(TriggerTiming::After, TriggerEvent::Insert, None)
        );
        assert_eq!(
            trigger_head("CREATE TRIGGER t INSTEAD OF UPDATE ON v BEGIN SELECT 1; END"),
            head(TriggerTiming::InsteadOf, TriggerEvent::Update, None)
        );
    }

    #[test]
    fn the_columns_of_an_update_and_the_schema_of_the_table_are_read() {
        assert_eq!(
            trigger_head(
                "CREATE TEMP TRIGGER IF NOT EXISTS \"main\".\"t\" AFTER UPDATE OF a, [b] \
                 ON \"main\".orders FOR EACH ROW BEGIN SELECT 1; END"
            ),
            TriggerHead {
                update_columns: vec!["a".into(), "b".into()],
                ..head(TriggerTiming::After, TriggerEvent::Update, Some("main"))
            }
        );
    }

    #[test]
    fn the_columns_of_an_update_keep_their_order_and_their_quoted_names() {
        // A comment sits in the list, and the quoted names contain a comma, a
        // keyword and a doubled quote mark.
        assert_eq!(
            trigger_head(
                "CREATE TRIGGER t UPDATE OF zeta, /* ON x */ \",\", \"on\", `a``b` -- , c\n\
                 ON orders BEGIN SELECT 1; END"
            ),
            TriggerHead {
                update_columns: vec!["zeta".into(), ",".into(), "on".into(), "a`b".into()],
                ..head(TriggerTiming::Before, TriggerEvent::Update, None)
            }
        );
    }

    #[test]
    fn an_update_without_a_column_list_gives_no_columns() {
        assert_eq!(
            trigger_head("CREATE TRIGGER t AFTER UPDATE ON orders BEGIN SELECT 1; END"),
            head(TriggerTiming::After, TriggerEvent::Update, None)
        );
        // A text that ends after the word OF gives an empty list.
        assert_eq!(
            trigger_head("CREATE TRIGGER t UPDATE OF"),
            head(TriggerTiming::Before, TriggerEvent::Update, None)
        );
    }

    #[test]
    fn a_quoted_name_is_never_a_keyword() {
        // The trigger is named after a keyword, and a comment and a literal
        // hold more keywords.
        assert_eq!(
            trigger_head(
                "CREATE TRIGGER `after` -- AFTER DELETE\n /* INSTEAD OF */ \
                 BEFORE INSERT ON 'odd''name'.\"it\"\"s\" BEGIN SELECT 'ON x.y'; END"
            ),
            head(
                TriggerTiming::Before,
                TriggerEvent::Insert,
                Some("odd'name")
            )
        );
    }

    #[test]
    fn a_text_that_is_not_a_trigger_gives_the_defaults() {
        let empty = TriggerHead {
            timing: TriggerTiming::Before,
            event: None,
            update_columns: Vec::new(),
            target_schema: None,
        };
        assert_eq!(trigger_head("CREATE TABLE t (a)"), empty);
        assert_eq!(trigger_head("CREATE TRIGGER"), empty);
        // A comment and a quoted name that do not end take the rest of the
        // text.
        assert_eq!(trigger_head("CREATE TRIGGER t /* AFTER"), empty);
        assert_eq!(trigger_head("CREATE TRIGGER \"t AFTER"), empty);
    }
}
