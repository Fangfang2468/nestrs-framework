//! 图展示用的类型名称缩短规则，只影响标签，不参与任何类型身份或候选选择。

use std::collections::{BTreeMap, BTreeSet};

use super::ValidatedGraph;

/// 短名相同时同时恢复双方的完整类型名，避免同名类型和不同泛型实参被误认。
pub(super) fn display_names(graph: &ValidatedGraph) -> BTreeMap<&'static str, String> {
    let mut groups = BTreeMap::<String, BTreeSet<&'static str>>::new();
    for node in &graph.nodes {
        for name in std::iter::once(node.identifier.service_type.name).chain(
            node.dependencies
                .iter()
                .map(|dependency| dependency.requested.service_type.name),
        ) {
            groups.entry(short_name(name)).or_default().insert(name);
        }
    }
    let mut names = BTreeMap::new();
    for (short, group) in groups {
        let collision = group.len() > 1;
        for name in group {
            names.insert(
                name,
                if collision {
                    name.to_owned()
                } else {
                    short.clone()
                },
            );
        }
    }
    names
}

/// 按标点分隔路径 token，可处理嵌套泛型、tuple、引用和 dyn trait，不递归解析类型。
fn short_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut characters = name.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        if character.is_alphanumeric() || character == '_' {
            let mut segment_start = start;
            let mut end = start + character.len_utf8();
            while let Some(&(index, next)) = characters.peek() {
                if next.is_alphanumeric() || next == '_' {
                    characters.next();
                    end = index + next.len_utf8();
                } else if next == ':' && name[index..].starts_with("::") {
                    let after_separator = index + 2;
                    if name[after_separator..]
                        .chars()
                        .next()
                        .is_some_and(|next| next.is_alphanumeric() || next == '_')
                    {
                        characters.next();
                        characters.next();
                        segment_start = after_separator;
                        end = after_separator;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            result.push_str(&name[segment_start..end]);
        } else {
            result.push(character);
        }
    }
    result
}
