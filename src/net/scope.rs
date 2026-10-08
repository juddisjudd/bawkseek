#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Everyone,
    User(String),
    Room(String),
}

/// Reads `@user query` or `#room query`, with quotes around names that contain spaces.
pub fn parse_scope(text: &str) -> (Scope, String) {
    let text = text.trim();
    let (kind, rest) = match text.chars().next() {
        Some('@') => ('@', &text[1..]),
        Some('#') => ('#', &text[1..]),
        _ => return (Scope::Everyone, text.to_string()),
    };
    let (name, query) = match rest.strip_prefix('"') {
        Some(quoted) => match quoted.split_once('"') {
            Some((name, query)) => (name, query),
            None => return (Scope::Everyone, text.to_string()),
        },
        None => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
    };
    let (name, query) = (name.trim(), query.trim());
    if name.is_empty() || query.is_empty() {
        return (Scope::Everyone, text.to_string());
    }
    let scope = if kind == '@' {
        Scope::User(name.to_string())
    } else {
        Scope::Room(name.to_string())
    };
    (scope, query.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_user_and_room_searches() {
        assert_eq!(
            parse_scope("aphex twin"),
            (Scope::Everyone, "aphex twin".into())
        );
        assert_eq!(
            parse_scope("@ann aphex twin"),
            (Scope::User("ann".into()), "aphex twin".into())
        );
        assert_eq!(
            parse_scope("#lossless flac"),
            (Scope::Room("lossless".into()), "flac".into())
        );
    }

    #[test]
    fn reads_quoted_names() {
        assert_eq!(
            parse_scope("#\"The Lobby\" flac 24"),
            (Scope::Room("The Lobby".into()), "flac 24".into())
        );
        assert_eq!(
            parse_scope("@\"dj shadow\" endtroducing"),
            (Scope::User("dj shadow".into()), "endtroducing".into())
        );
    }

    #[test]
    fn falls_back_to_everyone_without_a_query() {
        assert_eq!(parse_scope("@ann"), (Scope::Everyone, "@ann".into()));
        assert_eq!(parse_scope("#\"open"), (Scope::Everyone, "#\"open".into()));
    }
}
