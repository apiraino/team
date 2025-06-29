# TOML schema reference

## People

Every member of a Rust team is represented by a file in the `people` directory.
The file structure is this:

```toml
name = "John Doe"  # Real name of the person (required)
github = "johndoe"  # GitHub username of the person (required)
github-id = 123456  # GitHub ID of the person (required)
zulip-id = 123456   # Zulip ID of the person (optional)
discord-id = 123456 # Discord ID of the person (optional)
# You can also set `email = false` to explicitly disable the email for the user.
# This will, for example, avoid adding the person to the mailing lists.
email = "john@doe.com"  # Email address used for mailing lists (optional)
irc = "jdoe"  # Nickname of the person on IRC, if different than the GitHub one (optional)
matrix = "@john:doe.com" # Matrix username (MXID) of the person (optional)

[permissions]
# Optional, see the permissions documentation
```

The file must be named the same as the GitHub username.

## Teams

Each Rust team or working group is represented by a file in the `teams`
directory. The structure of the file is this:

```toml
name = "overlords"  # Name of the team, used for GitHub (required)
subteam-of = "gods"  # Name of the parent team of this team (optional)
# Set this to `true` if it is a top-level team, with a representative on the leadership-council.
top-level = true

# The kind of the team (optional). Could be:
# - team (default)
# - working-group
# - project-group
# - marker-team
kind = "working-group"

[people]
# Leads of the team, can be more than one and must be members of the team.
# Required, but it can be empty
leads = ["bors"]
# Members of the team, can be empty
members = [
    "bors",
    "rust-highfive",
    "rfcbot",
    "craterbot",
    "rust-timer",
    # Any subset of members may hold custom roles, beyond "Team leader" which is
    # controlled by the `leads` array above. Members with roles are written
    # using an inline table as follows. A simple string member like "bors" is
    # equivalent to {github = "bors", roles = []}. The strings in `roles` must
    # be present as the `id` of some role in [[roles]] section below.
    { github = "Crab01", roles = ["cohost"] },
    { github = "Crab02", roles = ["cohost"] },
]
# Past members of the team. They will not be considered as part of the team,
# but they will be recognized on the website.
#
# Most teams are required to have this alumni key, even if its value is an empty
# array. It is only optional in teams with kind="marker-team", and in teams
# which comprise only members of other teams via include-team-leads or similar.
alumni = [
    "buildbot",
]
# Optional, name of other teams whose members will be included as members of this team.
# Defaults to empty.
included-teams = []

# Include all members of all other teams. Optional, defaults to false.
# DO NOT USE, this is intended only for the `all` team.
# Include "all" in `included-teams` instead.
include-all-team-members = false
# Include all team leads. Optional, defaults to false.
# DO NOT USE, this is intended only for the `leads` team.
# Include "leads" in `included-teams` instead.
include-team-leads = false
# Include all working group leads. Optional, defaults to false.
# DO NOT USE, this is intended only for the `wg-leads` team.
# Include "wg-leads" in `included-teams` instead.
include-wg-leads = false
# Include all project group leads. Optional, defaults to false.
# DO NOT USE, this is intended only for the `project-group-leads` team.
# Include "project-group-leads" in `included-teams` instead.
include-project-group-leads = false
# Include all alumni. Optional, defaults to false.
# DO NOT USE, this is intended only for the `alumni` team.
# Include "alumni" in `included-teams` instead.
include-all-alumni = false

[permissions]
# Optional, applies to all team members. See the permissions documentation

[leads-permissions]
# Optional, applies only to team leads. See the permissions documentation

# Configure the GitHub integration
# This is optional, and if missing the team won't be synchronized with GitHub
[[github]]
team-name = "overlords-team"  # The name of the GitHub team (optional)
orgs = ["rust-lang"]  # Organizations to create the team in (required)
# Include members of these Rust teams in this GitHub team (optional)
extra-teams = ["bots-nursery"]

# Configures integration with rfcbot.
[rfcbot]
# The GitHub label to use for the team.
label = "T-cargo"
# The name of the team to be displayed by rfcbot.
name = "Cargo"
# The GitHub team to tag in a GitHub comment.
ping = "rust-lang/cargo"
# Team members to be excluded from FCPs
exclude-members = ["some-team-member"]

# Information about the team to display on the www.rust-lang.org website (required except marker teams).
[website]
# The name of the team to display on the website (required).
name = "Language team"
# A short description of the team (required).
description = "Designing and helping to implement new language features"
# The web page where this will appear, for example https://www.rust-lang.org/governance/teams/lang
# Defaults to the name of the team (defined at the top of this file).
# Subteams do not get a separate page. Only teams and working groups have pages.
page = "lang"
# The email address to contact the team.
email = "example@rust-lang.org"
# The GitHub repository where this team does their work.
repo = "http://github.com/rust-lang/lang-team"
# A link to access the team's Discord channel.
discord-invite = "https://discord.gg/e6Q3cvu"
# The name of the team's channel on Discord.
discord-name = "#wg-rustup"
# The name of the team's stream on Zulip.
zulip-stream = "t-lang"
# An alias for the team's matrix room.
matrix-room = "#t-lang:matrix.org"
# An integer to influence the sort order of team in the teams list.
# They are sorted in descending order, so very large positive values are
# first, and very negative values are last.
# Default is 0.
weight = -100

# Customized roles held by a subset of the team's members, beyond "Team leader"
# which is rendered automatically for members of the `leads` array.
[[roles]]
# Kebab-case id for the role. This serves as a key for translations.
id = "cohost"
# Text to appear on the website beneath the team member's name and GitHub handle.
description = "Co-host"

# Define the mailing lists used by the team
# It's optional, and there can be more than one
[[lists]]
# The email address of the list (required)
address = "overlords@rust-lang.org"
# This can be set to false to avoid including all the team members in the list
# It's useful if you want to create the list with a different set of members
# It's optional, and the default is `true`.
include-team-members = true
# Include all members of the team's subteams (optional - default `false`)
include-subteam-members = false
# Include the following extra people in the mailing list. Their email address
# will be fetched from their TOML in people/ (optional).
extra-people = [
    "alexcrichton",
]
# Include the following email addresses in the mailing list (optional).
extra-emails = [
    "noreply@rust-lang.org",
]
# Include all the members of the following teams in the mailing list
# (optional).
extra-teams = [
    "bots-nursery",
]

# Define the Zulip groups used by the team
# It's optional, and there can be more than one
[[zulip-groups]]
# The name of the Zulip group (required)
name = "T-overlords"
# This can be set to false to avoid including all the team members in the group
# It's useful if you want to create the group with a different set of members
# It's optional, and the default is `true`.
include-team-members = true
# Include the following extra people in the Zulip group. Their email address
# or Zulip id will be fetched from their TOML in people/ (optional).
extra-people = [
    "alexcrichton",
]
# Include the following Zulip ids in the Zulip group (optional).
extra-zulip-ids = [
    1234
]
# Include all the members of the following teams in the Zulip group
# (optional).
extra-teams = [
    "bots-nursery",
]
# Exclude the following people in the Zulip group (optional).
excluded-people = [
    "rylev",
]

# Define the Zulip streams used by the team
# It's optional, and there can be more than one.
#
# This will remove anyone who isn't in the team from the stream
# so it should only be used for private streams at the moment.
[[zulip-streams]]
# The name of the Zulip stream (required)
name = "t-overlords/private"
# This can be set to false to avoid including all the team members in the stream
# It's useful if you want to create the stream with a different set of members
# It's optional, and the default is `true`.
include-team-members = true
# Include the following extra people in the Zulip stream. Their email address
# or Zulip id will be fetched from their TOML in people/ (optional).
extra-people = [
    "alexcrichton",
]
# Include the following Zulip ids in the Zulip stream (optional).
extra-zulip-ids = [
    1234
]
# Include all the members of the following teams in the Zulip stream
# (optional).
extra-teams = [
    "bots-nursery",
]
# Exclude the following people in the Zulip stream (optional).
excluded-people = [
    "rylev",
]

# Roles to define in Discord.
[[discord-roles]]
# The name of the role.
name = "security"
# The color for the role.
color = "#e91e63"
```

## Permissions

Permissions can be applied either to a single person or to a whole team, and
they grant access to some pieces of rust-lang tooling. The following
permissions are available:

```toml
[permissions]
# All permissions are optional, including the `permissions` section

# Grants access to the @rust-timer GitHub bot
perf = true
# Grants access to the @craterbot GitHub bot
crater = true
# Grants admin access on crates.io
crates-io-admin = true
# Grants `@bors r+` rights in the repo `rust-lang/some-repo`
bors.some-repo.review = true
# Grants `@bors try` rights in the repo `rust-lang/some-repo`.
# This is a subset of `bors.some-repo.review`, so this shouldn't
# be set if `review` is also set.
bors.some-repo.try = true

# Access to the dev-desktop program.
# See https://forge.rust-lang.org/infra/docs/dev-desktop.html
dev-desktop = true
```

## Repos

Repos are configured by creating a file in the `repos` folder
under the corresponding org directory. For example, the `rust-lang/rust`
repository is managed by the file "repos/rust-lang/rust.toml".

The following sections describe the options available.[^missing]

If you need to transfer a repository into the `rust-lang/rust` org from outside the org, please consult with the Infrastructure Team. The general process is to get permission from infra to transfer it, and to coordinate transferring the repo (transfer requests expire after 24 hours, so be sure to coordinate accordingly). Then create a PR to the team repo to add the repository to set up permissions.

[^missing]: If particular GitHub settings are not mentioned here, consult with the Infrastructure Team to either have them manually make the changes, or to add support for the missing settings to the sync-team tool.

### General repository settings

```toml
# The org this repo belongs to (required)
org = "rust-lang"
# The name of the repo (required)
name = "my-repo"
# A description of the repo (required)
description = "A repo for awesome things!"
# A URL that is displayed next to the description.
homepage = "https://www.rust-lang.org/"
# The bots that this repo requires (required)
bots = ["bors", "rustbot", "rust-timer"]
# Should the repository be private? (optional - default `false`)
# Note that this only serves for documentation purposes, it is
# not synchronized by automation. If you need to create a private
# repository, please consult with the Infrastructure Team.
private-non-synced = false
```

### Repository access

Access to a repository is given on a per-team basis. Teams who are responsible for a repository may give access to other teams at their discretion.

See [GitHub's documentation][github-roles] for information on what each role is allowed to do. The recommendations for choosing a role are:

- `admin` — No users or teams except for org owners should have this permission level.
- `maintain` — Teams may have this permission level at their discretion for repositories the team is responsible for. Repositories using the bors bot may want to consider using the `write` permission level instead in order to deactivate the “Merge” button on PRs to enforce that merges go through bors.
- `write` — Teams that are responsible for a repository should have at least this permission level.
- `triage` — This role is available if teams want to give these permissions to other teams, such as for triage support. Unfortunately this role does not allow contributors to edit issue descriptions or titles, so its utility for that purpose is limited.

[github-roles]: https://help.github.com/en/github/setting-up-and-managing-organizations-and-teams/repository-permission-levels-for-an-organization

```toml
# The list of teams with access to this repository (required).
#
# The key is the team name, and the value is either:
# - "triage"
# - "write"
# - "maintain"
# - "admin"
[access.teams]
compiler = "write"

# Access granted to individuals. DO NOT USE! Access should only be given based
# on teams.
#
# The key is the GitHub username, and the value is the permission level (same as teams).
[access.individuals]
octocat = "write"
```

### Repository branch protections

[Branch protections] restrict actions on specified branches. It is strongly encouraged to set up branch protections on the default branch (e.g. `main` or `master`).

The behavior of branch protections depends on whether or not `bors` is enabled in the `bots` key mentioned above:

- If bors is not enabled, then the default will be to require at least one approving review (via GitHub's PR UI).
- If bors is enabled, approvals via GitHub's UI is not required (since we count the `@bors r+` comment as an approval). Also, bors will be added to the "allowed pushers".

Users with the "maintain" or "admin" role are allowed to merge PRs via the GitHub UI. If you have bors enabled, you should only give users the "write" role so that the "Merge" button is disabled, forcing the user to use the `@bors r+` comment instead.

The branch protection requires a PR to push changes. You cannot push directly to the branch.

Admins cannot override these branch protections. If an admin needs to do that, they will need to temporarily edit the branch protection in the GitHub settings.

[Branch protections]: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches

```toml
# The branch protections (optional)
[[branch-protections]]
# The pattern matching the branches to be protected (required)
pattern = "master"
# Which CI checks to are required for merging (optional)
# Cannot be set if `pr-required` is `false`.
ci-checks = ["CI"]
# Whether new commits after a reviewer's approval of a PR 
# merging into this branch require another review. 
# (optional - default `false`)
dismiss-stale-review = false
# Is a PR required when making changes to this branch?
# (optional - default `true`)
pr-required = true
# How many approvals are required for a PR to be merged.
# This option is only relevant if bors is not used.
# Cannot be set if `pr-required` is `false`.
# (optional - default `1`)
required-approvals = 1
# Which GitHub teams have access to push/merge to this branch.
# If unspecified, all teams/contributors with write or higher access
# can push/merge to the branch.
# Teams mentioned in this array must also have access to the repo
# in [access.teams].
# (optional)
allowed-merge-teams = ["awesome-team"]
# Determines the merge queue bot(s) that manage pushes to this branch.
# When a bot manages the queue, some other options, like
# `required-approvals` and `pr-required` options are not valid.
#
# Currently, only the "homu" option is supported.
# When "homu" is used, "bors" has to be in the `bots` array.
# (optional)
merge-bots = ["homu"]
```
