//! Folder and path projection for candidate files, in repository-relative and
//! rooted coordinates.

use smol_str::SmolStr;
use unlace_core::cluster::Partition;

use crate::analyze::relocation::PipelineSolver;
use crate::analyze::relocation::mirror::{PolishEvidence, ProjectedFolder};
use crate::narrate::{normalize_physical_path, project_physical_path};

impl PipelineSolver<'_> {
    pub(in crate::analyze::relocation) fn projected_folder_for_vertex(
        &self,
        parts: &Partition,
        vertex: u32,
        evidence: &PolishEvidence,
    ) -> Option<ProjectedFolder> {
        let scc = self.scc_of_vertex(vertex);
        if let Some(intended) = scc.and_then(|scc| evidence.applied_destination(scc)) {
            return Some(ProjectedFolder::Rooted(SmolStr::new(intended)));
        }
        if let Some(home) = self.retained_cycle_home(parts, vertex, evidence) {
            return Some(ProjectedFolder::RepositoryRelative(home.clone()));
        }
        scc.and_then(|scc| parts.cluster_of(scc))
            .and_then(|cluster| self.real_folder_names.get(cluster.0 as usize))
            .cloned()
            .map(ProjectedFolder::RepositoryRelative)
    }

    /// Original physical home only while the SCC has not moved or mirrored.
    pub(in crate::analyze::relocation) fn retained_cycle_home(
        &self,
        parts: &Partition,
        vertex: u32,
        evidence: &PolishEvidence,
    ) -> Option<&SmolStr> {
        let stayed = self.scc_of_vertex(vertex).is_some_and(|scc| {
            evidence.applied_destination(scc).is_none()
                && parts.cluster_of(scc).is_some_and(|cluster| {
                    self.pass_start_partition.cluster_of(scc) == Some(cluster)
                })
        });
        self.cycle_home_by_vertex.get(&vertex).filter(|_| stayed)
    }

    pub(in crate::analyze::relocation) fn repository_path(&self, path: &str) -> String {
        let relative: Vec<String> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        project_physical_path(self.root_name.as_str(), &relative).join("/")
    }

    pub(super) fn candidate_path(&self, path: &str) -> String {
        let path: Vec<String> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        normalize_physical_path(self.root_name.as_str(), &path).join("/")
    }
}
