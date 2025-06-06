mod api;
#[cfg(test)]
mod tests;

use self::api::{BranchProtectionOp, TeamPrivacy, TeamRole};
use crate::github::api::{GithubRead, Login, PushAllowanceActor, RepoPermission, RepoSettings};
use log::debug;
use rust_team_data::v1::{Bot, BranchProtectionMode, MergeBot};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub(crate) use self::api::{GitHubApiRead, GitHubWrite, HttpClient};

static DEFAULT_DESCRIPTION: &str = "Managed by the rust-lang/team repository.";
static DEFAULT_PRIVACY: TeamPrivacy = TeamPrivacy::Closed;

pub(crate) fn create_diff(
    github: Box<dyn GithubRead>,
    teams: Vec<rust_team_data::v1::Team>,
    repos: Vec<rust_team_data::v1::Repo>,
) -> anyhow::Result<Diff> {
    let github = SyncGitHub::new(github, teams, repos)?;
    github.diff_all()
}

type OrgName = String;

struct SyncGitHub {
    github: Box<dyn GithubRead>,
    teams: Vec<rust_team_data::v1::Team>,
    repos: Vec<rust_team_data::v1::Repo>,
    usernames_cache: HashMap<u64, String>,
    org_owners: HashMap<OrgName, HashSet<u64>>,
}

impl SyncGitHub {
    pub(crate) fn new(
        github: Box<dyn GithubRead>,
        teams: Vec<rust_team_data::v1::Team>,
        repos: Vec<rust_team_data::v1::Repo>,
    ) -> anyhow::Result<Self> {
        debug!("caching mapping between user ids and usernames");
        let users = teams
            .iter()
            .filter_map(|t| t.github.as_ref().map(|gh| &gh.teams))
            .flatten()
            .flat_map(|team| &team.members)
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let usernames_cache = github.usernames(&users)?;

        debug!("caching organization owners");
        let orgs = teams
            .iter()
            .filter_map(|t| t.github.as_ref())
            .flat_map(|gh| &gh.teams)
            .map(|gh_team| &gh_team.org)
            .collect::<HashSet<_>>();

        let mut org_owners = HashMap::new();

        for org in &orgs {
            org_owners.insert((*org).to_string(), github.org_owners(org)?);
        }

        Ok(SyncGitHub {
            github,
            teams,
            repos,
            usernames_cache,
            org_owners,
        })
    }

    pub(crate) fn diff_all(&self) -> anyhow::Result<Diff> {
        let team_diffs = self.diff_teams()?;
        let repo_diffs = self.diff_repos()?;

        Ok(Diff {
            team_diffs,
            repo_diffs,
        })
    }

    fn diff_teams(&self) -> anyhow::Result<Vec<TeamDiff>> {
        let mut diffs = Vec::new();
        let mut unseen_github_teams = HashMap::new();
        for team in &self.teams {
            if let Some(gh) = &team.github {
                for github_team in &gh.teams {
                    // Get existing teams we haven't seen yet
                    let unseen_github_teams = match unseen_github_teams.get_mut(&github_team.org) {
                        Some(ts) => ts,
                        None => {
                            let ts: HashMap<_, _> = self
                                .github
                                .org_teams(&github_team.org)?
                                .into_iter()
                                .collect();
                            unseen_github_teams
                                .entry(github_team.org.clone())
                                .or_insert(ts)
                        }
                    };
                    // Remove the current team from the collection of unseen GitHub teams
                    unseen_github_teams.remove(&github_team.name);

                    let diff_team = self.diff_team(github_team)?;
                    if !diff_team.noop() {
                        diffs.push(diff_team);
                    }
                }
            }
        }

        let delete_diffs = unseen_github_teams
            .into_iter()
            .filter(|(org, _)| matches!(org.as_str(), "rust-lang" | "rust-lang-nursery")) // Only delete unmanaged teams in `rust-lang` and `rust-lang-nursery` for now
            .flat_map(|(org, remaining_github_teams)| {
                remaining_github_teams
                    .into_iter()
                    .map(move |t| (org.clone(), t))
            })
            // Don't delete the special bot teams
            .filter(|(_, (remaining_github_team, _))| {
                !BOTS_TEAMS.contains(&remaining_github_team.as_str())
            })
            .map(|(org, (name, slug))| TeamDiff::Delete(DeleteTeamDiff { org, name, slug }));

        diffs.extend(delete_diffs);

        Ok(diffs)
    }

    fn diff_team(&self, github_team: &rust_team_data::v1::GitHubTeam) -> anyhow::Result<TeamDiff> {
        debug!("Diffing team `{}/{}`", github_team.org, github_team.name);

        // Ensure the team exists and is consistent
        let team = match self.github.team(&github_team.org, &github_team.name)? {
            Some(team) => team,
            None => {
                let members = github_team
                    .members
                    .iter()
                    .map(|member| {
                        let expected_role = self.expected_role(&github_team.org, *member);
                        (self.usernames_cache[member].clone(), expected_role)
                    })
                    .collect();
                return Ok(TeamDiff::Create(CreateTeamDiff {
                    org: github_team.org.clone(),
                    name: github_team.name.clone(),
                    description: DEFAULT_DESCRIPTION.to_owned(),
                    privacy: DEFAULT_PRIVACY,
                    members,
                }));
            }
        };
        let mut name_diff = None;
        if team.name != github_team.name {
            name_diff = Some(github_team.name.clone())
        }
        let mut description_diff = None;
        match &team.description {
            Some(description) => {
                if description != DEFAULT_DESCRIPTION {
                    description_diff = Some((description.clone(), DEFAULT_DESCRIPTION.to_owned()));
                }
            }
            None => {
                description_diff = Some((String::new(), DEFAULT_DESCRIPTION.to_owned()));
            }
        }
        let mut privacy_diff = None;
        if team.privacy != DEFAULT_PRIVACY {
            privacy_diff = Some((team.privacy, DEFAULT_PRIVACY))
        }

        let mut member_diffs = Vec::new();

        let mut current_members = self.github.team_memberships(&team, &github_team.org)?;
        let invites = self
            .github
            .team_membership_invitations(&github_team.org, &github_team.name)?;

        // Ensure all expected members are in the team
        for member in &github_team.members {
            let expected_role = self.expected_role(&github_team.org, *member);
            let username = &self.usernames_cache[member];
            if let Some(member) = current_members.remove(member) {
                if member.role != expected_role {
                    member_diffs.push((
                        username.clone(),
                        MemberDiff::ChangeRole((member.role, expected_role)),
                    ));
                } else {
                    member_diffs.push((username.clone(), MemberDiff::Noop));
                }
            } else {
                // Check if the user has been invited already
                if invites.contains(username) {
                    member_diffs.push((username.clone(), MemberDiff::Noop));
                } else {
                    member_diffs.push((username.clone(), MemberDiff::Create(expected_role)));
                }
            }
        }

        // The previous cycle removed expected members from current_members, so it only contains
        // members to delete now.
        for member in current_members.values() {
            member_diffs.push((member.username.clone(), MemberDiff::Delete));
        }

        Ok(TeamDiff::Edit(EditTeamDiff {
            org: github_team.org.clone(),
            name: team.name,
            name_diff,
            description_diff,
            privacy_diff,
            member_diffs,
        }))
    }

    fn diff_repos(&self) -> anyhow::Result<Vec<RepoDiff>> {
        let mut diffs = Vec::new();
        for repo in &self.repos {
            let repo_diff = self.diff_repo(repo)?;
            if !repo_diff.noop() {
                diffs.push(repo_diff);
            }
        }
        Ok(diffs)
    }

    fn diff_repo(&self, expected_repo: &rust_team_data::v1::Repo) -> anyhow::Result<RepoDiff> {
        debug!(
            "Diffing repo `{}/{}`",
            expected_repo.org, expected_repo.name
        );

        let actual_repo = match self.github.repo(&expected_repo.org, &expected_repo.name)? {
            Some(r) => r,
            None => {
                let permissions = calculate_permission_diffs(
                    expected_repo,
                    Default::default(),
                    Default::default(),
                )?;
                let mut branch_protections = Vec::new();
                for branch_protection in &expected_repo.branch_protections {
                    branch_protections.push((
                        branch_protection.pattern.clone(),
                        construct_branch_protection(expected_repo, branch_protection),
                    ));
                }

                return Ok(RepoDiff::Create(CreateRepoDiff {
                    org: expected_repo.org.clone(),
                    name: expected_repo.name.clone(),
                    settings: RepoSettings {
                        description: expected_repo.description.clone(),
                        homepage: expected_repo.homepage.clone(),
                        archived: false,
                        auto_merge_enabled: expected_repo.auto_merge_enabled,
                    },
                    permissions,
                    branch_protections,
                }));
            }
        };

        let permission_diffs = self.diff_permissions(expected_repo)?;
        let branch_protection_diffs = self.diff_branch_protections(&actual_repo, expected_repo)?;
        let old_settings = RepoSettings {
            description: actual_repo.description.clone(),
            homepage: actual_repo.homepage.clone(),
            archived: actual_repo.archived,
            auto_merge_enabled: actual_repo.allow_auto_merge.unwrap_or(false),
        };
        let new_settings = RepoSettings {
            description: expected_repo.description.clone(),
            homepage: expected_repo.homepage.clone(),
            archived: expected_repo.archived,
            auto_merge_enabled: expected_repo.auto_merge_enabled,
        };

        Ok(RepoDiff::Update(UpdateRepoDiff {
            org: expected_repo.org.clone(),
            name: actual_repo.name,
            repo_node_id: actual_repo.node_id,
            settings_diff: (old_settings, new_settings),
            permission_diffs,
            branch_protection_diffs,
        }))
    }

    fn diff_permissions(
        &self,
        expected_repo: &rust_team_data::v1::Repo,
    ) -> anyhow::Result<Vec<RepoPermissionAssignmentDiff>> {
        let actual_teams: HashMap<_, _> = self
            .github
            .repo_teams(&expected_repo.org, &expected_repo.name)?
            .into_iter()
            .map(|t| (t.name.clone(), t))
            .collect();
        let actual_collaborators: HashMap<_, _> = self
            .github
            .repo_collaborators(&expected_repo.org, &expected_repo.name)?
            .into_iter()
            .map(|u| (u.name.clone(), u))
            .collect();

        calculate_permission_diffs(expected_repo, actual_teams, actual_collaborators)
    }

    fn diff_branch_protections(
        &self,
        actual_repo: &api::Repo,
        expected_repo: &rust_team_data::v1::Repo,
    ) -> anyhow::Result<Vec<BranchProtectionDiff>> {
        // The rust-lang/rust repository uses GitHub apps push allowance actors for its branch
        // protections, which cannot be read without a PAT.
        // To avoid errors, we simply return an empty diff here.
        if !self.github.uses_pat() && actual_repo.org == "rust-lang" && actual_repo.name == "rust" {
            return Ok(vec![]);
        }

        let mut branch_protection_diffs = Vec::new();
        let mut actual_protections = self
            .github
            .branch_protections(&actual_repo.org, &actual_repo.name)?;
        for branch_protection in &expected_repo.branch_protections {
            let actual_branch_protection = actual_protections.remove(&branch_protection.pattern);
            let mut expected_branch_protection =
                construct_branch_protection(expected_repo, branch_protection);

            // We don't model GitHub App push allowance actors in team.
            // However, we don't want to remove existing accesses of GH apps to
            // branches.
            // So if there is an existing branch protection, we copy its GitHub app
            // push allowances into the expected branch protection, to roundtrip the app access.
            if let Some((_, actual_branch_protection)) = &actual_branch_protection {
                expected_branch_protection.push_allowances.extend(
                    actual_branch_protection
                        .push_allowances
                        .iter()
                        .filter(|allowance| matches!(allowance, PushAllowanceActor::App(_)))
                        .cloned(),
                );
            }

            let operation = {
                match actual_branch_protection {
                    Some((database_id, bp)) if bp != expected_branch_protection => {
                        BranchProtectionDiffOperation::Update(
                            database_id,
                            bp,
                            expected_branch_protection,
                        )
                    }
                    None => BranchProtectionDiffOperation::Create(expected_branch_protection),
                    // The branch protection doesn't need to change
                    Some(_) => continue,
                }
            };
            branch_protection_diffs.push(BranchProtectionDiff {
                pattern: branch_protection.pattern.clone(),
                operation,
            })
        }

        // `actual_branch_protections` now contains the branch protections that were not expected
        // but are still on GitHub. We want to delete them.
        branch_protection_diffs.extend(actual_protections.into_iter().map(|(name, (id, _))| {
            BranchProtectionDiff {
                pattern: name,
                operation: BranchProtectionDiffOperation::Delete(id),
            }
        }));

        Ok(branch_protection_diffs)
    }

    fn expected_role(&self, org: &str, user: u64) -> TeamRole {
        if let Some(true) = self
            .org_owners
            .get(org)
            .map(|owners| owners.contains(&user))
        {
            TeamRole::Maintainer
        } else {
            TeamRole::Member
        }
    }
}

fn calculate_permission_diffs(
    expected_repo: &rust_team_data::v1::Repo,
    mut actual_teams: HashMap<String, api::RepoTeam>,
    mut actual_collaborators: HashMap<String, api::RepoUser>,
) -> anyhow::Result<Vec<RepoPermissionAssignmentDiff>> {
    let mut permissions = Vec::new();
    // Team permissions
    for expected_team in &expected_repo.teams {
        let permission = convert_permission(&expected_team.permission);
        let actual_team = actual_teams.remove(&expected_team.name);
        let collaborator = RepoCollaborator::Team(expected_team.name.clone());

        let diff = match actual_team {
            Some(t) if t.permission != permission => RepoPermissionAssignmentDiff {
                collaborator,
                diff: RepoPermissionDiff::Update(t.permission, permission),
            },
            // Team permission does not need to change
            Some(_) => continue,
            None => RepoPermissionAssignmentDiff {
                collaborator,
                diff: RepoPermissionDiff::Create(permission),
            },
        };
        permissions.push(diff);
    }
    // Bot permissions
    let bots = expected_repo.bots.iter().filter_map(|b| {
        let bot_user_name = bot_user_name(b)?;
        actual_teams.remove(bot_user_name);
        Some((bot_user_name, RepoPermission::Write))
    });
    // Member permissions
    let members = expected_repo
        .members
        .iter()
        .map(|m| (m.name.as_str(), convert_permission(&m.permission)));
    for (name, permission) in bots.chain(members) {
        let actual_collaborator = actual_collaborators.remove(name);
        let collaborator = RepoCollaborator::User(name.to_owned());
        let diff = match actual_collaborator {
            Some(t) if t.permission != permission => RepoPermissionAssignmentDiff {
                collaborator,
                diff: RepoPermissionDiff::Update(t.permission, permission),
            },
            // Collaborator permission does not need to change
            Some(_) => continue,
            None => RepoPermissionAssignmentDiff {
                collaborator,
                diff: RepoPermissionDiff::Create(permission),
            },
        };
        permissions.push(diff);
    }
    // `actual_teams` now contains the teams that were not expected
    // but are still on GitHub. We now remove them.
    for (team, t) in actual_teams {
        if t.name == "security" && expected_repo.org == "rust-lang" {
            // Skip removing access permissions from security.
            // If we're in this branch we know that the team repo doesn't mention this team at all,
            // so this shouldn't remove intentionally granted non-read access.  Security is granted
            // read access to all repositories in the org by GitHub (via a "security manager"
            // role), and we can't remove that access.
            //
            // (FIXME: If we find security with non-read access, *that* probably should get dropped
            // to read access. But not worth doing in this commit, want to get us unblocked first).
            continue;
        }
        permissions.push(RepoPermissionAssignmentDiff {
            collaborator: RepoCollaborator::Team(team),
            diff: RepoPermissionDiff::Delete(t.permission),
        });
    }
    // `actual_collaborators` now contains the collaborators that were not expected
    // but are still on GitHub. We now remove them.
    for (collaborator, u) in actual_collaborators {
        permissions.push(RepoPermissionAssignmentDiff {
            collaborator: RepoCollaborator::User(collaborator),
            diff: RepoPermissionDiff::Delete(u.permission),
        });
    }
    Ok(permissions)
}

/// Returns `None` if the bot is not an actual bot user, but rather a GitHub app.
fn bot_user_name(bot: &Bot) -> Option<&str> {
    match bot {
        // FIXME: set this to `None` once homu is removed completely
        Bot::Bors => Some("bors"),
        Bot::Highfive => Some("rust-highfive"),
        Bot::RustTimer => Some("rust-timer"),
        Bot::Rustbot => Some("rustbot"),
        Bot::Rfcbot => Some("rfcbot"),
        Bot::Craterbot => Some("craterbot"),
        Bot::Glacierbot => Some("rust-lang-glacier-bot"),
        Bot::LogAnalyzer => Some("rust-log-analyzer"),
        Bot::Renovate => None,
    }
}

pub fn convert_permission(p: &rust_team_data::v1::RepoPermission) -> RepoPermission {
    use rust_team_data::v1;
    match *p {
        v1::RepoPermission::Write => RepoPermission::Write,
        v1::RepoPermission::Admin => RepoPermission::Admin,
        v1::RepoPermission::Maintain => RepoPermission::Maintain,
        v1::RepoPermission::Triage => RepoPermission::Triage,
    }
}

pub fn construct_branch_protection(
    expected_repo: &rust_team_data::v1::Repo,
    branch_protection: &rust_team_data::v1::BranchProtection,
) -> api::BranchProtection {
    let uses_merge_bot = !branch_protection.merge_bots.is_empty();
    // When a merge bot manages a branch, we should not require a PR nor approvals
    // for that branch, because it will (force) push to these branches directly.
    let branch_protection_mode = if uses_merge_bot {
        BranchProtectionMode::PrNotRequired
    } else {
        branch_protection.mode.clone()
    };

    let required_approving_review_count: u8 = match branch_protection_mode {
        BranchProtectionMode::PrRequired {
            required_approvals, ..
        } => required_approvals
            .try_into()
            .expect("Too large required approval count"),
        BranchProtectionMode::PrNotRequired => 0,
    };
    let mut push_allowances: Vec<PushAllowanceActor> = branch_protection
        .allowed_merge_teams
        .iter()
        .map(|team| {
            api::PushAllowanceActor::Team(api::TeamPushAllowanceActor {
                organization: Login {
                    login: expected_repo.org.clone(),
                },
                name: team.to_string(),
            })
        })
        .collect();

    for merge_bot in &branch_protection.merge_bots {
        let allowance = match merge_bot {
            MergeBot::Homu => PushAllowanceActor::User(api::UserPushAllowanceActor {
                login: "bors".to_owned(),
            }),
            MergeBot::RustTimer => PushAllowanceActor::User(api::UserPushAllowanceActor {
                login: "rust-timer".to_owned(),
            }),
        };
        push_allowances.push(allowance);
    }

    let mut checks = match &branch_protection_mode {
        BranchProtectionMode::PrRequired { ci_checks, .. } => ci_checks.clone(),
        BranchProtectionMode::PrNotRequired => {
            vec![]
        }
    };
    // Normalize check order to avoid diffs based only on the ordering difference
    checks.sort();

    api::BranchProtection {
        pattern: branch_protection.pattern.clone(),
        is_admin_enforced: true,
        dismisses_stale_reviews: branch_protection.dismiss_stale_review,
        required_approving_review_count,
        required_status_check_contexts: checks,
        push_allowances,
        requires_approving_reviews: matches!(
            branch_protection_mode,
            BranchProtectionMode::PrRequired { .. }
        ),
    }
}

/// The special bot teams
const BOTS_TEAMS: &[&str] = &["bors", "highfive", "rfcbot", "bots"];

/// A diff between the team repo and the state on GitHub
pub(crate) struct Diff {
    team_diffs: Vec<TeamDiff>,
    repo_diffs: Vec<RepoDiff>,
}

impl Diff {
    /// Apply the diff to GitHub
    pub(crate) fn apply(self, sync: &GitHubWrite) -> anyhow::Result<()> {
        for team_diff in self.team_diffs {
            team_diff.apply(sync)?;
        }
        for repo_diff in self.repo_diffs {
            repo_diff.apply(sync)?;
        }

        Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.team_diffs.is_empty() && self.repo_diffs.is_empty()
    }
}

impl std::fmt::Display for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.team_diffs.is_empty() {
            writeln!(f, "💻 Team Diffs:")?;
            for team_diff in &self.team_diffs {
                write!(f, "{team_diff}")?;
            }
        }

        if !&self.repo_diffs.is_empty() {
            writeln!(f, "💻 Repo Diffs:")?;
            for repo_diff in &self.repo_diffs {
                write!(f, "{repo_diff}")?;
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
enum RepoDiff {
    Create(CreateRepoDiff),
    Update(UpdateRepoDiff),
}

impl RepoDiff {
    fn apply(&self, sync: &GitHubWrite) -> anyhow::Result<()> {
        match self {
            RepoDiff::Create(c) => c.apply(sync),
            RepoDiff::Update(u) => u.apply(sync),
        }
    }

    fn noop(&self) -> bool {
        match self {
            RepoDiff::Create(_c) => false,
            RepoDiff::Update(u) => u.noop(),
        }
    }
}

impl std::fmt::Display for RepoDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create(c) => write!(f, "{c}"),
            Self::Update(u) => write!(f, "{u}"),
        }
    }
}

#[derive(Debug)]
struct CreateRepoDiff {
    org: String,
    name: String,
    settings: RepoSettings,
    permissions: Vec<RepoPermissionAssignmentDiff>,
    branch_protections: Vec<(String, api::BranchProtection)>,
}

impl CreateRepoDiff {
    fn apply(&self, sync: &GitHubWrite) -> anyhow::Result<()> {
        let repo = sync.create_repo(&self.org, &self.name, &self.settings)?;

        for permission in &self.permissions {
            permission.apply(sync, &self.org, &self.name)?;
        }

        for (branch, protection) in &self.branch_protections {
            BranchProtectionDiff {
                pattern: branch.clone(),
                operation: BranchProtectionDiffOperation::Create(protection.clone()),
            }
            .apply(sync, &self.org, &self.name, &repo.node_id)?;
        }

        Ok(())
    }
}

impl std::fmt::Display for CreateRepoDiff {
    fn fmt(&self, mut f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let CreateRepoDiff {
            org,
            name,
            settings,
            permissions,
            branch_protections,
        } = self;

        let RepoSettings {
            description,
            homepage,
            archived: _,
            auto_merge_enabled,
        } = &settings;

        writeln!(f, "➕ Creating repo:")?;
        writeln!(f, "  Org: {org}")?;
        writeln!(f, "  Name: {name}")?;
        writeln!(f, "  Description: {description}")?;
        writeln!(f, "  Homepage: {homepage:?}")?;
        writeln!(f, "  Auto-merge: {auto_merge_enabled}")?;
        writeln!(f, "  Permissions:")?;
        for diff in permissions {
            write!(f, "{diff}")?;
        }
        writeln!(f, "  Branch Protections:")?;
        for (branch_name, branch_protection) in branch_protections {
            writeln!(&mut f, "    {branch_name}")?;
            log_branch_protection(branch_protection, None, &mut f)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct UpdateRepoDiff {
    org: String,
    name: String,
    repo_node_id: String,
    // old, new
    settings_diff: (RepoSettings, RepoSettings),
    permission_diffs: Vec<RepoPermissionAssignmentDiff>,
    branch_protection_diffs: Vec<BranchProtectionDiff>,
}

impl UpdateRepoDiff {
    pub(crate) fn noop(&self) -> bool {
        if !self.can_be_modified() {
            return true;
        }

        let UpdateRepoDiff {
            org: _,
            name: _,
            repo_node_id: _,
            settings_diff,
            permission_diffs,
            branch_protection_diffs,
        } = self;

        settings_diff.0 == settings_diff.1
            && permission_diffs.is_empty()
            && branch_protection_diffs.is_empty()
    }

    fn can_be_modified(&self) -> bool {
        // Archived repositories cannot be modified
        // If the repository should be archived, and we do not change its archival status,
        // we should not change any other properties of the repo.
        if self.settings_diff.1.archived && self.settings_diff.0.archived {
            return false;
        }
        true
    }

    fn apply(&self, sync: &GitHubWrite) -> anyhow::Result<()> {
        if !self.can_be_modified() {
            return Ok(());
        }

        // If we're unarchiving, we have to unarchive first and *then* modify other properties
        // of the repository. On the other hand, if we're achiving, we need to perform
        // the archiving *last* (otherwise permissions and branch protections cannot be modified)
        // anymore. If we're not changing the archival status, the order doesn't really matter.
        let is_unarchive = self.settings_diff.0.archived && !self.settings_diff.1.archived;

        if is_unarchive {
            sync.edit_repo(&self.org, &self.name, &self.settings_diff.1)?;
        }

        for permission in &self.permission_diffs {
            permission.apply(sync, &self.org, &self.name)?;
        }

        for branch_protection in &self.branch_protection_diffs {
            branch_protection.apply(sync, &self.org, &self.name, &self.repo_node_id)?;
        }

        if !is_unarchive && self.settings_diff.0 != self.settings_diff.1 {
            sync.edit_repo(&self.org, &self.name, &self.settings_diff.1)?;
        }

        Ok(())
    }
}

impl std::fmt::Display for UpdateRepoDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.noop() {
            return Ok(());
        }

        let UpdateRepoDiff {
            org,
            name,
            repo_node_id: _,
            settings_diff,
            permission_diffs,
            branch_protection_diffs,
        } = self;

        writeln!(f, "📝 Editing repo '{org}/{name}':")?;
        let (settings_old, settings_new) = &settings_diff;
        let RepoSettings {
            description,
            homepage,
            archived,
            auto_merge_enabled,
        } = settings_old;
        match (description.as_str(), settings_new.description.as_str()) {
            ("", "") => {}
            ("", new) => writeln!(f, "  Set description: '{new}'")?,
            (old, "") => writeln!(f, "  Remove description: '{old}'")?,
            (old, new) if old != new => writeln!(f, "  New description: '{old}' => '{new}'")?,
            _ => {}
        }
        match (homepage, &settings_new.homepage) {
            (None, Some(new)) => writeln!(f, "  Set homepage: '{new}'")?,
            (Some(old), None) => writeln!(f, "  Remove homepage: '{old}'")?,
            (Some(old), Some(new)) if old != new => {
                writeln!(f, "  New homepage: '{old}' => '{new}'")?
            }
            _ => {}
        }
        match (archived, &settings_new.archived) {
            (false, true) => writeln!(f, "  Archive")?,
            (true, false) => writeln!(f, "  Unarchive")?,
            _ => {}
        }
        match (auto_merge_enabled, &settings_new.auto_merge_enabled) {
            (false, true) => writeln!(f, "  Enable auto-merge")?,
            (true, false) => writeln!(f, "  Disable auto-merge")?,
            _ => {}
        }
        if !permission_diffs.is_empty() {
            writeln!(f, "  Permission Changes:")?;
            for permission_diff in permission_diffs {
                write!(f, "{permission_diff}")?;
            }
        }
        if !branch_protection_diffs.is_empty() {
            writeln!(f, "  Branch Protections:")?;
            for branch_protection_diff in branch_protection_diffs {
                write!(f, "{branch_protection_diff}")?;
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct RepoPermissionAssignmentDiff {
    collaborator: RepoCollaborator,
    diff: RepoPermissionDiff,
}

impl RepoPermissionAssignmentDiff {
    fn apply(&self, sync: &GitHubWrite, org: &str, repo_name: &str) -> anyhow::Result<()> {
        match &self.diff {
            RepoPermissionDiff::Create(p) | RepoPermissionDiff::Update(_, p) => {
                match &self.collaborator {
                    RepoCollaborator::Team(team_name) => {
                        sync.update_team_repo_permissions(org, repo_name, team_name, p)?
                    }
                    RepoCollaborator::User(user_name) => {
                        sync.update_user_repo_permissions(org, repo_name, user_name, p)?
                    }
                }
            }
            RepoPermissionDiff::Delete(_) => match &self.collaborator {
                RepoCollaborator::Team(team_name) => {
                    sync.remove_team_from_repo(org, repo_name, team_name)?
                }
                RepoCollaborator::User(user_name) => {
                    sync.remove_collaborator_from_repo(org, repo_name, user_name)?
                }
            },
        }
        Ok(())
    }
}

impl std::fmt::Display for RepoPermissionAssignmentDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let RepoPermissionAssignmentDiff { collaborator, diff } = self;

        let name = match &collaborator {
            RepoCollaborator::Team(name) => format!("team '{name}'"),
            RepoCollaborator::User(name) => format!("user '{name}'"),
        };
        match &diff {
            RepoPermissionDiff::Create(p) => {
                writeln!(f, "    Giving {name} {p} permission")
            }
            RepoPermissionDiff::Update(old, new) => {
                writeln!(f, "    Changing {name}'s permission from {old} to {new}")
            }
            RepoPermissionDiff::Delete(p) => {
                writeln!(f, "    Removing {name}'s {p} permission ")
            }
        }
    }
}

#[derive(Debug)]
enum RepoPermissionDiff {
    Create(RepoPermission),
    Update(RepoPermission, RepoPermission),
    Delete(RepoPermission),
}

#[derive(Clone, Debug)]
enum RepoCollaborator {
    Team(String),
    User(String),
}

#[derive(Debug)]
struct BranchProtectionDiff {
    pattern: String,
    operation: BranchProtectionDiffOperation,
}

impl BranchProtectionDiff {
    fn apply(
        &self,
        sync: &GitHubWrite,
        org: &str,
        repo_name: &str,
        repo_id: &str,
    ) -> anyhow::Result<()> {
        match &self.operation {
            BranchProtectionDiffOperation::Create(bp) => {
                sync.upsert_branch_protection(
                    BranchProtectionOp::CreateForRepo(repo_id.to_string()),
                    &self.pattern,
                    bp,
                    org,
                )?;
            }
            BranchProtectionDiffOperation::Update(id, _, bp) => {
                sync.upsert_branch_protection(
                    BranchProtectionOp::UpdateBranchProtection(id.clone()),
                    &self.pattern,
                    bp,
                    org,
                )?;
            }
            BranchProtectionDiffOperation::Delete(id) => {
                debug!(
                    "Deleting branch protection '{}' on '{}/{}' as \
                the protection is not in the team repo",
                    self.pattern, org, repo_name
                );
                sync.delete_branch_protection(org, repo_name, id)?;
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for BranchProtectionDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "      {}", self.pattern)?;
        match &self.operation {
            BranchProtectionDiffOperation::Create(bp) => log_branch_protection(bp, None, f),
            BranchProtectionDiffOperation::Update(_, old, new) => {
                log_branch_protection(old, Some(new), f)
            }
            BranchProtectionDiffOperation::Delete(_) => {
                writeln!(f, "        Deleting branch protection")
            }
        }
    }
}

fn log_branch_protection(
    current: &api::BranchProtection,
    new: Option<&api::BranchProtection>,
    mut result: impl Write,
) -> std::fmt::Result {
    let api::BranchProtection {
        // Pattern identifies the branch protection, so it has to be same between `current`
        // and `new`.
        pattern: _,
        is_admin_enforced,
        dismisses_stale_reviews,
        required_approving_review_count,
        required_status_check_contexts,
        push_allowances,
        requires_approving_reviews,
    } = current;

    macro_rules! log {
        ($str:literal, $field1:ident) => {
            let old = $field1;
            let new = new.map(|n| &n.$field1);
            log!($str, old, new);
        };
        ($str:literal, $old:expr, $new:expr) => {
            if Some($old) != $new {
                if let Some(n) = $new.as_ref() {
                    writeln!(result, "        {}: {:?} => {:?}", $str, $old, n)?;
                } else {
                    writeln!(result, "        {}: {:?}", $str, $old)?;
                };
            }
        };
    }

    log!("Dismiss Stale Reviews", dismisses_stale_reviews);
    log!("Is admin enforced", is_admin_enforced);
    log!(
        "Required Approving Review Count",
        required_approving_review_count
    );
    log!("Requires PR", requires_approving_reviews);
    log!("Required Checks", required_status_check_contexts);
    log!("Allowances", push_allowances);
    Ok(())
}

#[derive(Debug)]
enum BranchProtectionDiffOperation {
    Create(api::BranchProtection),
    Update(String, api::BranchProtection, api::BranchProtection),
    Delete(String),
}

#[derive(Debug)]
enum TeamDiff {
    Create(CreateTeamDiff),
    Edit(EditTeamDiff),
    Delete(DeleteTeamDiff),
}

impl TeamDiff {
    fn apply(self, sync: &GitHubWrite) -> anyhow::Result<()> {
        match self {
            TeamDiff::Create(c) => c.apply(sync)?,
            TeamDiff::Edit(e) => e.apply(sync)?,
            TeamDiff::Delete(d) => d.apply(sync)?,
        }

        Ok(())
    }

    fn noop(&self) -> bool {
        match self {
            TeamDiff::Create(_) | TeamDiff::Delete(_) => false,
            TeamDiff::Edit(e) => e.noop(),
        }
    }
}

impl std::fmt::Display for TeamDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeamDiff::Create(c) => write!(f, "{c}"),
            TeamDiff::Edit(e) => write!(f, "{e}"),
            TeamDiff::Delete(d) => write!(f, "{d}"),
        }
    }
}

#[derive(Debug)]
struct CreateTeamDiff {
    org: String,
    name: String,
    description: String,
    privacy: TeamPrivacy,
    members: Vec<(String, TeamRole)>,
}

impl CreateTeamDiff {
    fn apply(self, sync: &GitHubWrite) -> anyhow::Result<()> {
        sync.create_team(&self.org, &self.name, &self.description, self.privacy)?;
        for (member_name, role) in self.members {
            MemberDiff::Create(role).apply(&self.org, &self.name, &member_name, sync)?;
        }

        Ok(())
    }
}

impl std::fmt::Display for CreateTeamDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let CreateTeamDiff {
            org,
            name,
            description,
            privacy,
            members,
        } = self;

        writeln!(f, "➕ Creating team:")?;
        writeln!(f, "  Org: {org}")?;
        writeln!(f, "  Name: {name}")?;
        writeln!(f, "  Description: {description}")?;
        writeln!(
            f,
            "  Privacy: {}",
            match privacy {
                TeamPrivacy::Secret => "secret",
                TeamPrivacy::Closed => "closed",
            }
        )?;
        writeln!(f, "  Members:")?;
        for (name, role) in members {
            writeln!(f, "    {name}: {role}")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct EditTeamDiff {
    org: String,
    name: String,
    name_diff: Option<String>,
    description_diff: Option<(String, String)>,
    privacy_diff: Option<(TeamPrivacy, TeamPrivacy)>,
    member_diffs: Vec<(String, MemberDiff)>,
}

impl EditTeamDiff {
    fn apply(self, sync: &GitHubWrite) -> anyhow::Result<()> {
        if self.name_diff.is_some()
            || self.description_diff.is_some()
            || self.privacy_diff.is_some()
        {
            sync.edit_team(
                &self.org,
                &self.name,
                self.name_diff.as_deref(),
                self.description_diff.as_ref().map(|(_, d)| d.as_str()),
                self.privacy_diff.map(|(_, p)| p),
            )?;
        }

        for (member_name, member_diff) in self.member_diffs {
            member_diff.apply(&self.org, &self.name, &member_name, sync)?;
        }

        Ok(())
    }

    fn noop(&self) -> bool {
        let EditTeamDiff {
            org: _,
            name: _,
            name_diff,
            description_diff,
            privacy_diff,
            member_diffs,
        } = self;

        name_diff.is_none()
            && description_diff.is_none()
            && privacy_diff.is_none()
            && member_diffs.iter().all(|(_, d)| d.is_noop())
    }
}

impl std::fmt::Display for EditTeamDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.noop() {
            return Ok(());
        }

        let EditTeamDiff {
            org,
            name,
            name_diff,
            description_diff,
            privacy_diff,
            member_diffs,
        } = self;

        writeln!(f, "📝 Editing team '{org}/{name}':")?;
        if let Some(n) = name_diff {
            writeln!(f, "  New name: {n}")?;
        }
        if let Some((old, new)) = &description_diff {
            writeln!(f, "  New description: '{old}' => '{new}'")?;
        }
        if let Some((old, new)) = &privacy_diff {
            let display = |privacy: &TeamPrivacy| match privacy {
                TeamPrivacy::Secret => "secret",
                TeamPrivacy::Closed => "closed",
            };
            writeln!(f, "  New privacy: '{}' => '{}'", display(old), display(new))?;
        }
        for (member, diff) in member_diffs {
            match diff {
                MemberDiff::Create(r) => {
                    writeln!(f, "  Adding member '{member}' with {r} role")?;
                }
                MemberDiff::ChangeRole((o, n)) => {
                    writeln!(f, "  Changing '{member}' role from {o} to {n}")?;
                }
                MemberDiff::Delete => {
                    writeln!(f, "  Deleting member '{member}'")?;
                }
                MemberDiff::Noop => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
enum MemberDiff {
    Create(TeamRole),
    ChangeRole((TeamRole, TeamRole)),
    Delete,
    Noop,
}

impl MemberDiff {
    fn apply(self, org: &str, team: &str, member: &str, sync: &GitHubWrite) -> anyhow::Result<()> {
        match self {
            MemberDiff::Create(role) | MemberDiff::ChangeRole((_, role)) => {
                sync.set_team_membership(org, team, member, role)?;
            }
            MemberDiff::Delete => sync.remove_team_membership(org, team, member)?,
            MemberDiff::Noop => {}
        }

        Ok(())
    }

    fn is_noop(&self) -> bool {
        matches!(self, Self::Noop)
    }
}

#[derive(Debug)]
struct DeleteTeamDiff {
    org: String,
    name: String,
    slug: String,
}

impl DeleteTeamDiff {
    fn apply(self, sync: &GitHubWrite) -> anyhow::Result<()> {
        sync.delete_team(&self.org, &self.slug)?;
        Ok(())
    }
}

impl std::fmt::Display for DeleteTeamDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "❌ Deleting team '{}/{}'", self.org, self.name)?;
        Ok(())
    }
}
