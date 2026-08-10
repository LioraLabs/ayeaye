//! What this build of ayeaye is, rendered for a human.

/// A build's self-description: the version it claims, and the capabilities that
/// were compiled into it.
///
/// The capabilities are supplied by the caller rather than detected here — the
/// shell knows what it linked, and the core only knows how to say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity<'a> {
    /// The version string, e.g. `0.2.2`.
    pub version: &'a str,
    /// Short labels for what this build can do, e.g. `cpu`, `metal`.
    pub capabilities: &'a [&'a str],
}

impl Identity<'_> {
    /// One line naming the version and, when there are any, the capabilities.
    pub fn banner(&self) -> String {
        if self.capabilities.is_empty() {
            format!("ayeaye {}", self.version)
        } else {
            format!("ayeaye {} ({})", self.version, self.capabilities.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Identity;

    // AYEAYE-41
    #[test]
    fn banner_names_the_version_and_the_capabilities() {
        let id = Identity {
            version: "1.2.3",
            capabilities: &["cpu", "metal"],
        };
        assert_eq!(id.banner(), "ayeaye 1.2.3 (cpu, metal)");
    }

    // AYEAYE-41
    #[test]
    fn banner_omits_the_parentheses_when_there_are_no_capabilities() {
        let id = Identity {
            version: "1.2.3",
            capabilities: &[],
        };
        assert_eq!(id.banner(), "ayeaye 1.2.3");
    }
}
