use super::WaitingChildren;

#[test]
fn sparse_pending_children_keep_their_original_slots_and_zero_task_id() {
    let mut children = WaitingChildren::Empty;
    assert!(children.iter().next().is_none());
    children.insert(7, 0, 64);
    assert_eq!(children.iter().collect::<Vec<_>>(), vec![(7, 0)]);
    assert_eq!(children.take(6), None);
    assert_eq!(children.take(7), Some(0));
    assert!(children.iter().next().is_none());
    children.insert(63, 4, 64);
    children.insert(3, 4, 64);
    children.insert(22, 9, 64);
    assert_eq!(children.take(63), Some(4));
    assert_eq!(children.take(63), None);
    assert_eq!(children.iter().collect::<Vec<_>>(), vec![(3, 4), (22, 9)]);
    assert_eq!(children.take(3), Some(4));
    assert_eq!(children.take(22), Some(9));
    assert!(children.iter().next().is_none());
}
