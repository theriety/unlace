//! Join permissions and candidate acceptance checks of the identity guard.

use std::collections::BTreeSet;

use smol_str::SmolStr;
use unlace_core::cluster::{ClusterId, Partition};

use crate::analyze::relocation::identity_guard::{
    IdentitySubgroup, RelocationIdentityGuard, RenderIdentity,
};
use crate::analyze::relocation::mirror::PolishEvidence;

impl RelocationIdentityGuard {
    pub(in crate::analyze::relocation) fn permits_join(
        &self,
        parts: &Partition,
        moving: u32,
        target: ClusterId,
    ) -> bool {
        let Some(moving_identities) = self.identities_by_scc.get(moving as usize) else {
            return false;
        };
        let Some(allowed) = self.allowed_namespaces.get(target.0 as usize) else {
            return false;
        };
        if !self.permits_package_join(moving, target) || !self.permits_zone_join(moving, target) {
            return false;
        }
        if moving_identities
            .iter()
            .any(|(namespace, _)| !allowed.contains(namespace))
        {
            return false;
        }
        let mut occupied = BTreeSet::new();
        for (scc, identities) in self.identities_by_scc.iter().enumerate() {
            let scc = u32::try_from(scc).unwrap_or(u32::MAX);
            if scc == moving || parts.cluster_of(scc) != Some(target) {
                continue;
            }
            occupied.extend(identities.iter().cloned());
        }
        let unique: BTreeSet<&(SmolStr, SmolStr)> = moving_identities.iter().collect();
        unique.len() == moving_identities.len()
            && moving_identities
                .iter()
                .all(|identity| !occupied.contains(identity))
    }

    /// Permits a pinned test follower to join a production-owned logical
    /// cluster while retaining its own render namespace. Exact mirror rules
    /// establish that cross-namespace relationship; leaf collisions remain a
    /// hard veto within the projected namespace. The package wall is checked
    /// earlier through [`Self::permits_package_join`], so a follower blocked
    /// by it reports `packageBoundary` rather than a path collision.
    pub(in crate::analyze::relocation) fn permits_shadow_join(
        &self,
        parts: &Partition,
        moving: u32,
        target: ClusterId,
    ) -> bool {
        let Some(moving_identities) = self.identities_by_scc.get(moving as usize) else {
            return false;
        };
        let mut occupied = BTreeSet::new();
        for (scc, identities) in self.identities_by_scc.iter().enumerate() {
            let scc = u32::try_from(scc).unwrap_or(u32::MAX);
            if scc == moving || parts.cluster_of(scc) != Some(target) {
                continue;
            }
            occupied.extend(identities.iter().cloned());
        }
        let unique: BTreeSet<&RenderIdentity> = moving_identities.iter().collect();
        unique.len() == moving_identities.len()
            && moving_identities
                .iter()
                .all(|identity| !occupied.contains(identity))
    }

    #[cfg(test)]
    pub(in crate::analyze) fn accepts(&self, parts: &Partition) -> bool {
        self.accepts_with_mirrors(parts, &PolishEvidence::default())
    }

    /// Checks a finished candidate against every pass-start permission: each
    /// SCC sits in a cluster that admits its namespaces (applied mirror
    /// followers excepted) and, with the wall up, its packages; and no two
    /// files collide on one rendered identity.
    pub(in crate::analyze) fn accepts_with_mirrors(
        &self,
        parts: &Partition,
        evidence: &PolishEvidence,
    ) -> bool {
        let applied = evidence.applied_sccs();
        let mut occupied: BTreeSet<(ClusterId, &SmolStr, &SmolStr)> = BTreeSet::new();
        self.identities_by_scc
            .iter()
            .enumerate()
            .all(|(scc, identities)| {
                let scc = u32::try_from(scc).unwrap_or(u32::MAX);
                let Some(cluster) = parts.cluster_of(scc) else {
                    return false;
                };
                let Some(allowed) = self.allowed_namespaces.get(cluster.0 as usize) else {
                    return false;
                };
                if !self.permits_package_join(scc, cluster)
                    || !(applied.contains(&scc) || self.permits_zone_join(scc, cluster))
                {
                    return false;
                }
                identities.iter().all(|(namespace, leaf)| {
                    (allowed.contains(namespace) || applied.contains(&scc))
                        && occupied.insert((cluster, namespace, leaf))
                })
            })
    }

    /// Splits `group` into collision-free subgroups; with the package wall up
    /// each subgroup also holds a single package, so a new group folder never
    /// spans packages, and an SCC spanning packages joins no subgroup at all
    /// (it stays in its pass-start cluster).
    pub(in crate::analyze) fn collision_free_subgroups(&self, group: &[u32]) -> Vec<Vec<u32>> {
        let mut subgroups: Vec<IdentitySubgroup> = Vec::new();
        for &scc in group {
            let Some(identities) = self.identities_by_scc.get(scc as usize) else {
                continue;
            };
            let unique: BTreeSet<RenderIdentity> = identities.iter().cloned().collect();
            if unique.len() != identities.len()
                || (self.package_wall && !self.is_single_package(scc))
            {
                continue;
            }
            let packages = self.packages_by_scc.get(scc as usize);
            if let Some((members, occupied)) = subgroups.iter_mut().find(|(members, occupied)| {
                occupied.is_disjoint(&unique)
                    && members.first().is_some_and(|&first| {
                        self.zones_by_scc.get(first as usize) == self.zones_by_scc.get(scc as usize)
                    })
                    && (!self.package_wall
                        || members.first().is_some_and(|&first| {
                            self.packages_by_scc.get(first as usize) == packages
                        }))
            }) {
                members.push(scc);
                occupied.extend(unique);
            } else {
                subgroups.push((vec![scc], unique));
            }
        }
        subgroups.into_iter().map(|(members, _)| members).collect()
    }
}

#[cfg(test)]
mod tests {
    use smol_str::SmolStr;
    use unlace_core::cluster::{ClusterId, Partition};

    use crate::analyze::relocation::identity_guard::RelocationIdentityGuard;
    use crate::analyze::relocation::{FileInfo, LaminarHome};
    use crate::analyze::test_support::singleton_condensation;

    fn info(path: &str) -> FileInfo {
        FileInfo {
            container: 0,
            name: SmolStr::new(path),
            production_sloc: 1,
            namespace: SmolStr::default(),
            home: LaminarHome {
                folder: SmolStr::default(),
                domain: SmolStr::default(),
                package: SmolStr::new("p"),
                synthetic: false,
                label_keyed: false,
            },
        }
    }

    /// Three production files of one package, each alone in its own folder.
    fn identity() -> Partition {
        Partition::from_assignment((0..3).map(ClusterId).collect(), 3)
    }

    fn guard() -> RelocationIdentityGuard {
        let files = [info("a/a.ts"), info("b/b.ts"), info("c/c.ts")];
        RelocationIdentityGuard::new(
            &files,
            &singleton_condensation(3),
            &identity(),
            &identity(),
            None,
        )
    }

    #[test]
    fn should_refuse_a_join_by_an_unknown_scc_or_into_an_unknown_cluster() {
        let guard = guard();
        let parts = identity();

        assert!(guard.permits_join(&parts, 0, ClusterId(1)));
        assert!(!guard.permits_join(&parts, 9, ClusterId(1)));
        assert!(!guard.permits_join(&parts, 0, ClusterId(9)));
    }

    #[test]
    fn should_refuse_a_shadow_join_by_an_unknown_scc() {
        let guard = guard();
        let parts = identity();

        assert!(guard.permits_shadow_join(&parts, 0, ClusterId(1)));
        assert!(!guard.permits_shadow_join(&parts, 9, ClusterId(1)));
    }

    #[test]
    fn should_reject_a_candidate_leaving_an_scc_unplaced() {
        let guard = guard();
        let unplaced = Partition::from_assignment(vec![ClusterId(0), ClusterId(1)], 3);

        assert!(guard.accepts(&identity()));
        assert!(!guard.accepts(&unplaced));
    }

    #[test]
    fn should_reject_a_candidate_placing_an_scc_in_a_cluster_unknown_at_pass_start() {
        let guard = guard();
        let stray = Partition::from_assignment(vec![ClusterId(0), ClusterId(1), ClusterId(9)], 10);

        assert!(!guard.accepts(&stray));
    }

    #[test]
    fn should_leave_an_unknown_scc_out_of_every_collision_free_subgroup() {
        let guard = guard();

        assert_eq!(guard.collision_free_subgroups(&[9, 0, 1]), vec![vec![0, 1]]);
    }
}
