use super::*;

pub(super) fn systems_conflict(left: &Entry, right: &Entry) -> bool {
    // Conditions have dynamic accesses unknown to the scheduler, and exclusive systems receive
    // unrestricted &mut World. Both are conservative barriers against every system.
    if left.condition.is_some()
        || right.condition.is_some()
        || matches!(&left.kind, SystemKind::Exclusive(_))
        || matches!(&right.kind, SystemKind::Exclusive(_))
    {
        return true;
    }

    left.accesses.iter().any(|left_access| {
        right.accesses.iter().any(|right_access| {
            left_access.key == right_access.key
                && (left_access.mode == AccessMode::Write || right_access.mode == AccessMode::Write)
        })
    })
}

pub(super) fn topological_order(
    stage: Stage,
    members: &[usize],
    edges: &BTreeMap<usize, BTreeSet<usize>>,
    entries: &[Entry],
) -> Result<Vec<usize>, ScheduleError> {
    let mut indegree: BTreeMap<usize, usize> = members.iter().map(|&index| (index, 0)).collect();
    for targets in edges.values() {
        for &target in targets {
            *indegree
                .get_mut(&target)
                .expect("edge target is a stage member") += 1;
        }
    }
    let mut ready: BTreeSet<_> = indegree
        .iter()
        .filter_map(|(&index, &degree)| (degree == 0).then_some(index))
        .collect();
    let mut sorted = Vec::with_capacity(members.len());
    while let Some(index) = ready.pop_first() {
        sorted.push(index);
        for &target in edges.get(&index).expect("stage member edge exists") {
            let degree = indegree
                .get_mut(&target)
                .expect("edge target is a stage member");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(target);
            }
        }
    }
    if sorted.len() != members.len() {
        return Err(ScheduleError::Cycle {
            stage,
            systems: indegree
                .iter()
                .filter(|(_, degree)| **degree > 0)
                .map(|(&index, _)| entries[index].name.clone())
                .collect(),
        });
    }
    Ok(sorted)
}

pub(super) fn topological_batches(
    stage: Stage,
    members: &[usize],
    edges: &BTreeMap<usize, BTreeSet<usize>>,
    entries: &[Entry],
) -> Result<(Vec<usize>, Vec<Vec<usize>>), ScheduleError> {
    let mut indegree: BTreeMap<usize, usize> = members.iter().map(|&index| (index, 0)).collect();
    for targets in edges.values() {
        for &target in targets {
            *indegree
                .get_mut(&target)
                .expect("edge target is a stage member") += 1;
        }
    }
    let mut ready: BTreeSet<_> = indegree
        .iter()
        .filter_map(|(&index, &degree)| (degree == 0).then_some(index))
        .collect();
    let mut order = Vec::with_capacity(members.len());
    let mut batches = Vec::new();

    while !ready.is_empty() {
        // The whole ready frontier can run together: all explicit dependencies are satisfied and
        // every conflicting pair has already been serialized with an automatic dependency edge.
        let batch: Vec<_> = ready.iter().copied().collect();
        ready.clear();
        let mut next_ready = BTreeSet::new();
        for &index in &batch {
            order.push(index);
            for &target in edges.get(&index).expect("stage member edge exists") {
                let degree = indegree
                    .get_mut(&target)
                    .expect("edge target is a stage member");
                *degree -= 1;
                if *degree == 0 {
                    next_ready.insert(target);
                }
            }
        }
        batches.push(batch);
        ready = next_ready;
    }

    if order.len() != members.len() {
        return Err(ScheduleError::Cycle {
            stage,
            systems: indegree
                .iter()
                .filter(|(_, degree)| **degree > 0)
                .map(|(&index, _)| entries[index].name.clone())
                .collect(),
        });
    }
    Ok((order, batches))
}

impl AccessDeclaration {
    pub(super) fn target_description(self) -> String {
        let kind = match self.key.kind {
            AccessKind::Component => "component",
            AccessKind::Resource => "resource",
        };
        format!("{kind} `{}`", self.key.type_name)
    }
}
