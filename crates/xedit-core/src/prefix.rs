/// Prefix area commands — typed into the line number area
#[derive(Debug, Clone, PartialEq)]
pub enum PrefixCommand {
    /// `/` — Make this the current line
    SetCurrent,
    /// `d` — Delete this line
    Delete,
    /// `dd` — Start/end block delete
    DeleteBlock,
    /// `i[n]` — Insert n blank lines after (default 1)
    Insert(usize),
    /// `a[n]` — Add n blank lines after (default 1)
    Add(usize),
    /// `c` — Copy this line (needs destination)
    Copy,
    /// `cc` — Start/end block copy
    CopyBlock,
    /// `m` — Move this line (needs destination)
    Move,
    /// `mm` — Start/end block move
    MoveBlock,
    /// `"[n]` — Duplicate this line n times (default 1)
    Duplicate(usize),
    /// `""` — Start/end block duplicate
    DuplicateBlock,
    /// `f` — Following: destination marker (insert after this line)
    Following,
    /// `p` — Preceding: destination marker (insert before this line)
    Preceding,
    /// `>[n]` — Shift line right n columns (default 2)
    ShiftRight(usize),
    /// `<[n]` — Shift line left n columns (default 2)
    ShiftLeft(usize),
}

/// Types of block operations
#[derive(Debug, Clone, PartialEq)]
pub enum BlockType {
    Delete,
    Copy,
    Move,
    Duplicate,
}

/// A pending block operation waiting for the closing marker
#[derive(Debug, Clone)]
pub struct PendingBlock {
    pub command: BlockType,
    pub start_line: usize,
}

/// A pending copy/move needing a destination
#[derive(Debug, Clone)]
pub struct PendingOperation {
    pub op_type: OperationType,
    pub source_start: usize,
    pub source_end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperationType {
    Copy,
    Move,
}

impl PrefixCommand {
    /// Parse a prefix command from text typed in the prefix area
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        let lower = input.to_lowercase();

        match lower.as_str() {
            "/" => Some(PrefixCommand::SetCurrent),
            "d" => Some(PrefixCommand::Delete),
            "dd" => Some(PrefixCommand::DeleteBlock),
            "c" => Some(PrefixCommand::Copy),
            "cc" => Some(PrefixCommand::CopyBlock),
            "m" => Some(PrefixCommand::Move),
            "mm" => Some(PrefixCommand::MoveBlock),
            "f" => Some(PrefixCommand::Following),
            "p" => Some(PrefixCommand::Preceding),
            "\"" => Some(PrefixCommand::Duplicate(1)),
            "\"\"" => Some(PrefixCommand::DuplicateBlock),
            ">" => Some(PrefixCommand::ShiftRight(2)),
            "<" => Some(PrefixCommand::ShiftLeft(2)),
            "i" | "a" => Some(PrefixCommand::Insert(1)),
            _ => {
                if let Some(rest) = lower.strip_prefix('i') {
                    if let Ok(n) = rest.parse::<usize>() {
                        return Some(PrefixCommand::Insert(n));
                    }
                }
                if let Some(rest) = lower.strip_prefix('a') {
                    if let Ok(n) = rest.parse::<usize>() {
                        return Some(PrefixCommand::Add(n));
                    }
                }
                if let Some(rest) = lower.strip_prefix('"') {
                    if let Ok(n) = rest.parse::<usize>() {
                        return Some(PrefixCommand::Duplicate(n));
                    }
                }
                if let Some(rest) = lower.strip_prefix('>') {
                    if let Ok(n) = rest.parse::<usize>() {
                        return Some(PrefixCommand::ShiftRight(n));
                    }
                }
                if let Some(rest) = lower.strip_prefix('<') {
                    if let Ok(n) = rest.parse::<usize>() {
                        return Some(PrefixCommand::ShiftLeft(n));
                    }
                }
                None
            }
        }
    }

    pub fn is_block_marker(&self) -> bool {
        matches!(
            self,
            PrefixCommand::DeleteBlock
                | PrefixCommand::CopyBlock
                | PrefixCommand::MoveBlock
                | PrefixCommand::DuplicateBlock
        )
    }

    pub fn block_type(&self) -> Option<BlockType> {
        match self {
            PrefixCommand::DeleteBlock => Some(BlockType::Delete),
            PrefixCommand::CopyBlock => Some(BlockType::Copy),
            PrefixCommand::MoveBlock => Some(BlockType::Move),
            PrefixCommand::DuplicateBlock => Some(BlockType::Duplicate),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_commands() {
        assert_eq!(PrefixCommand::parse("/"), Some(PrefixCommand::SetCurrent));
        assert_eq!(PrefixCommand::parse("d"), Some(PrefixCommand::Delete));
        assert_eq!(PrefixCommand::parse("dd"), Some(PrefixCommand::DeleteBlock));
        assert_eq!(PrefixCommand::parse("i"), Some(PrefixCommand::Insert(1)));
        assert_eq!(PrefixCommand::parse("i5"), Some(PrefixCommand::Insert(5)));
        assert_eq!(PrefixCommand::parse("m"), Some(PrefixCommand::Move));
        assert_eq!(PrefixCommand::parse("mm"), Some(PrefixCommand::MoveBlock));
    }

    #[test]
    fn parse_shift() {
        assert_eq!(
            PrefixCommand::parse(">"),
            Some(PrefixCommand::ShiftRight(2))
        );
        assert_eq!(
            PrefixCommand::parse(">4"),
            Some(PrefixCommand::ShiftRight(4))
        );
        assert_eq!(PrefixCommand::parse("<"), Some(PrefixCommand::ShiftLeft(2)));
    }

    #[test]
    fn parse_empty() {
        assert_eq!(PrefixCommand::parse(""), None);
        assert_eq!(PrefixCommand::parse("   "), None);
    }
}
