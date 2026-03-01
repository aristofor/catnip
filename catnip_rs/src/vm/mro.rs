// FILE: catnip_rs/src/vm/mro.rs
//! C3 linearization for multiple inheritance (MRO).
//!
//! Implements the C3 linearization algorithm used by Python.
//! Reference: https://www.python.org/download/releases/2.3/mro/

/// Compute the C3 linearization for a type with the given direct parents.
///
/// - `name`: the type being linearized
/// - `parents`: direct parent names, left-to-right
/// - `get_mro`: closure that returns the MRO of a given parent (must already be computed)
///
/// Returns `Ok(mro)` with the full MRO (including `name` itself), or `Err(msg)` on failure.
pub fn c3_linearize<F>(name: &str, parents: &[String], get_mro: F) -> Result<Vec<String>, String>
where
    F: Fn(&str) -> Option<Vec<String>>,
{
    if parents.is_empty() {
        return Ok(vec![name.to_string()]);
    }

    // Build the list of sequences to merge:
    // L[parent] for each parent, then the list of parents itself
    let mut sequences: Vec<Vec<String>> = Vec::with_capacity(parents.len() + 1);
    for p in parents {
        let parent_mro = get_mro(p).ok_or_else(|| format!("Unknown base struct '{p}'"))?;
        sequences.push(parent_mro);
    }
    sequences.push(parents.to_vec());

    let mut result = vec![name.to_string()];

    loop {
        // Remove empty sequences
        sequences.retain(|s| !s.is_empty());
        if sequences.is_empty() {
            break;
        }

        // Find a good head: first element of some sequence that doesn't appear
        // in the tail of any other sequence
        let mut found = None;
        for seq in &sequences {
            let candidate = &seq[0];
            let in_tail = sequences
                .iter()
                .any(|s| s.len() > 1 && s[1..].contains(candidate));
            if !in_tail {
                found = Some(candidate.clone());
                break;
            }
        }

        let head = found.ok_or_else(|| {
            format!(
                "Cannot create a consistent MRO for '{}': C3 linearization failed",
                name
            )
        })?;

        result.push(head.clone());

        // Remove head from the front of all sequences
        for seq in &mut sequences {
            if !seq.is_empty() && seq[0] == head {
                seq.remove(0);
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_registry(entries: &[(&str, Vec<String>)]) -> HashMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_no_parents() {
        let mro = c3_linearize("A", &[], |_| None).unwrap();
        assert_eq!(mro, vec!["A"]);
    }

    #[test]
    fn test_single_parent() {
        let reg = make_registry(&[("B", vec!["B".into()])]);
        let mro = c3_linearize("A", &["B".into()], |n| reg.get(n).cloned()).unwrap();
        assert_eq!(mro, vec!["A", "B"]);
    }

    #[test]
    fn test_diamond() {
        // A
        // B(A)  C(A)
        // D(B, C)
        let reg = make_registry(&[
            ("A", vec!["A".into()]),
            ("B", vec!["B".into(), "A".into()]),
            ("C", vec!["C".into(), "A".into()]),
        ]);
        let mro = c3_linearize("D", &["B".into(), "C".into()], |n| reg.get(n).cloned()).unwrap();
        assert_eq!(mro, vec!["D", "B", "C", "A"]);
    }

    #[test]
    fn test_linear_chain() {
        // A -> B -> C
        let reg = make_registry(&[("A", vec!["A".into()]), ("B", vec!["B".into(), "A".into()])]);
        let mro = c3_linearize("C", &["B".into()], |n| reg.get(n).cloned()).unwrap();
        assert_eq!(mro, vec!["C", "B", "A"]);
    }

    #[test]
    fn test_inconsistent_hierarchy() {
        // A(X, Y) and B(Y, X) -> D(A, B) should fail
        let reg = make_registry(&[
            ("X", vec!["X".into()]),
            ("Y", vec!["Y".into()]),
            ("A", vec!["A".into(), "X".into(), "Y".into()]),
            ("B", vec!["B".into(), "Y".into(), "X".into()]),
        ]);
        let result = c3_linearize("D", &["A".into(), "B".into()], |n| reg.get(n).cloned());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("C3 linearization failed"));
    }

    #[test]
    fn test_unknown_parent() {
        let result = c3_linearize("A", &["Unknown".into()], |_| None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown base struct"));
    }

    #[test]
    fn test_left_priority() {
        // B and C are independent, D(B, C) -> B comes before C
        let reg = make_registry(&[("B", vec!["B".into()]), ("C", vec!["C".into()])]);
        let mro = c3_linearize("D", &["B".into(), "C".into()], |n| reg.get(n).cloned()).unwrap();
        assert_eq!(mro, vec!["D", "B", "C"]);
    }

    #[test]
    fn test_complex_diamond() {
        // Python classic example:
        //     O
        //    / \
        //   A   B
        //   |   |
        //   C   D
        //    \ /
        //     E
        let reg = make_registry(&[
            ("O", vec!["O".into()]),
            ("A", vec!["A".into(), "O".into()]),
            ("B", vec!["B".into(), "O".into()]),
            ("C", vec!["C".into(), "A".into(), "O".into()]),
            ("D", vec!["D".into(), "B".into(), "O".into()]),
        ]);
        let mro = c3_linearize("E", &["C".into(), "D".into()], |n| reg.get(n).cloned()).unwrap();
        assert_eq!(mro, vec!["E", "C", "A", "D", "B", "O"]);
    }
}
