#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

LINEAR_URL = "https://api.linear.app/graphql"


def load_hermes_env() -> None:
    home = os.path.expanduser("~")
    env_path = os.path.join(home, ".hermes", ".env")
    if not os.path.exists(env_path):
        return
    try:
        with open(env_path, "r", encoding="utf-8") as handle:
            for raw in handle:
                line = raw.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, value = line.split("=", 1)
                if key and key not in os.environ:
                    os.environ[key] = value
    except OSError:
        return


def env_key(name: str) -> str:
    load_hermes_env()
    value = os.getenv(name, "").strip()
    if not value:
        print(f"{name} is not set.", file=sys.stderr)
        raise SystemExit(2)
    return value


def post_graphql(query: str, *, variables: dict[str, Any] | None = None) -> dict[str, Any]:
    body: dict[str, Any] = {"query": query}
    if variables:
        body["variables"] = variables
    request = Request(
        LINEAR_URL,
        data=json.dumps(body).encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "Authorization": env_key("LINEAR_API_KEY"),
            "User-Agent": "finite-linear-finite/1.0",
        },
        method="POST",
    )
    try:
        with urlopen(request, timeout=20) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except HTTPError as exc:
        payload = exc.read().decode("utf-8", errors="replace")
        print(payload or f"HTTP {exc.code}", file=sys.stderr)
        raise SystemExit(exc.code)
    except URLError as exc:
        print(str(exc), file=sys.stderr)
        raise SystemExit(1)

    if payload.get("errors"):
        print(json.dumps(payload["errors"], indent=2), file=sys.stderr)
        raise SystemExit(1)
    return payload.get("data") or {}


def priority_label(value: Any) -> str:
    mapping = {
        0: "none",
        1: "urgent",
        2: "high",
        3: "medium",
        4: "low",
    }
    try:
        return mapping.get(int(value), str(value))
    except (TypeError, ValueError):
        return str(value or "?")


def print_json(value: Any) -> int:
    print(json.dumps(value, indent=2))
    return 0


def filter_clause(*, team_key: str | None, assignee_email: str | None, state_type: str | None) -> str:
    fields: list[str] = []
    if team_key:
        fields.append(f"team: {{ key: {{ eq: {json.dumps(team_key)} }} }}")
    if assignee_email:
        fields.append(f"assignee: {{ email: {{ eq: {json.dumps(assignee_email)} }} }}")
    if state_type:
        fields.append(f"state: {{ type: {{ in: [{json.dumps(state_type)}] }} }}")
    if not fields:
        return ""
    return ", filter: { " + ", ".join(fields) + " }"


def cmd_viewer(args: argparse.Namespace) -> int:
    data = post_graphql("query { viewer { id name email active } }")
    viewer = data.get("viewer") or {}
    if args.json:
        return print_json(viewer)
    print(f"{viewer.get('name', '?')} <{viewer.get('email', '?')}>")
    print(f"id: {viewer.get('id', '?')} | active: {viewer.get('active', '?')}")
    return 0


def cmd_teams(args: argparse.Namespace) -> int:
    data = post_graphql("query { teams { nodes { id name key } } }")
    teams = (data.get("teams") or {}).get("nodes") or []
    if args.json:
        return print_json(teams)
    for team in teams:
        print(f"{team.get('key', '?'):>8}  {team.get('name', '?')}  ({team.get('id', '?')})")
    return 0


def cmd_workflow_states(args: argparse.Namespace) -> int:
    data = post_graphql(
        """
        query($teamKey: String!) {
          workflowStates(filter: { team: { key: { eq: $teamKey } } }) {
            nodes { id name type }
          }
        }
        """,
        variables={"teamKey": args.team_key},
    )
    states = (data.get("workflowStates") or {}).get("nodes") or []
    if args.json:
        return print_json(states)
    for state in states:
        print(f"{state.get('type', '?'):>10}  {state.get('name', '?')}  ({state.get('id', '?')})")
    return 0


def cmd_issues(args: argparse.Namespace) -> int:
    clause = filter_clause(
        team_key=args.team_key,
        assignee_email=args.assignee_email,
        state_type=args.state_type,
    )
    data = post_graphql(
        f"""
        query {{
          issues(first: {args.limit}{clause}) {{
            nodes {{
              id identifier title priority
              state {{ id name type }}
              assignee {{ id name email }}
              team {{ id key name }}
              url
            }}
          }}
        }}
        """
    )
    issues = (data.get("issues") or {}).get("nodes") or []
    if args.json:
        return print_json(issues)
    for issue in issues:
        assignee = (issue.get("assignee") or {}).get("name") or "unassigned"
        state = issue.get("state") or {}
        print(f"{issue.get('identifier', '?'):>10}  [{state.get('name', '?')}]  {issue.get('title', '?')}")
        print(
            f"            team={((issue.get('team') or {}).get('key') or '?')} "
            f"assignee={assignee} priority={priority_label(issue.get('priority'))}"
        )
        if issue.get("url"):
            print(f"            {issue['url']}")
    return 0


def cmd_issue(args: argparse.Namespace) -> int:
    data = post_graphql(
        """
        query($issueId: String!) {
          issue(id: $issueId) {
            id identifier title description priority url dueDate
            state { id name type }
            assignee { id name email }
            team { id key name }
            project { id name }
            labels { nodes { id name color } }
            comments { nodes { id body createdAt user { name } } }
          }
        }
        """,
        variables={"issueId": args.issue_id},
    )
    issue = data.get("issue") or {}
    if args.json:
        return print_json(issue)
    print(f"{issue.get('identifier', '?')}  {issue.get('title', '?')}")
    print(f"state: {(issue.get('state') or {}).get('name', '?')} | priority: {priority_label(issue.get('priority'))}")
    print(f"team: {(issue.get('team') or {}).get('key', '?')} | assignee: {((issue.get('assignee') or {}).get('name') or 'unassigned')}")
    if issue.get("project"):
        print(f"project: {(issue['project'] or {}).get('name', '?')}")
    if issue.get("dueDate"):
        print(f"due: {issue['dueDate']}")
    if issue.get("url"):
        print(issue["url"])
    if issue.get("description"):
        print("")
        print(issue["description"])
    comments = (issue.get("comments") or {}).get("nodes") or []
    if comments:
        print("\nComments:")
        for comment in comments[:10]:
            user = (comment.get("user") or {}).get("name") or "unknown"
            body = (comment.get("body") or "").strip().replace("\n", " ")
            print(f"- {user}: {body[:200]}")
    return 0


def cmd_issue_search(args: argparse.Namespace) -> int:
    data = post_graphql(
        """
        query($query: String!, $limit: Int!) {
          issueSearch(query: $query, first: $limit) {
            nodes {
              id identifier title url priority
              state { name type }
              assignee { name email }
            }
          }
        }
        """,
        variables={"query": args.query, "limit": args.limit},
    )
    issues = (data.get("issueSearch") or {}).get("nodes") or []
    if args.json:
        return print_json(issues)
    for issue in issues:
        assignee = (issue.get("assignee") or {}).get("name") or "unassigned"
        print(f"{issue.get('identifier', '?'):>10}  [{(issue.get('state') or {}).get('name', '?')}]  {issue.get('title', '?')}")
        print(f"            assignee={assignee} priority={priority_label(issue.get('priority'))}")
        if issue.get("url"):
            print(f"            {issue['url']}")
    return 0


def cmd_projects(args: argparse.Namespace) -> int:
    data = post_graphql(
        """
        query($limit: Int!) {
          projects(first: $limit) {
            nodes { id name description progress url lead { name } teams { nodes { key } } }
          }
        }
        """,
        variables={"limit": args.limit},
    )
    projects = (data.get("projects") or {}).get("nodes") or []
    if args.json:
        return print_json(projects)
    for project in projects:
        teams = ", ".join(node.get("key", "?") for node in ((project.get("teams") or {}).get("nodes") or []))
        print(f"{project.get('name', '?')} ({project.get('id', '?')})")
        print(f"  lead={((project.get('lead') or {}).get('name') or 'n/a')} progress={project.get('progress', '?')} teams={teams or 'n/a'}")
        if project.get("url"):
            print(f"  {project['url']}")
    return 0


def cmd_users(args: argparse.Namespace) -> int:
    data = post_graphql("query { users { nodes { id name email active } } }")
    users = (data.get("users") or {}).get("nodes") or []
    if args.json:
        return print_json(users)
    for user in users:
        print(f"{user.get('name', '?')} <{user.get('email', '?')}> ({user.get('id', '?')}) active={user.get('active', '?')}")
    return 0


def cmd_labels(args: argparse.Namespace) -> int:
    data = post_graphql("query { issueLabels { nodes { id name color } } }")
    labels = (data.get("issueLabels") or {}).get("nodes") or []
    if args.json:
        return print_json(labels)
    for label in labels:
        print(f"{label.get('name', '?'):>20}  {label.get('color', '?')}  ({label.get('id', '?')})")
    return 0


def cmd_create_issue(args: argparse.Namespace) -> int:
    data = post_graphql(
        """
        mutation($input: IssueCreateInput!) {
          issueCreate(input: $input) {
            success
            issue { id identifier title url }
          }
        }
        """,
        variables={
            "input": {
                "teamId": args.team_id,
                "title": args.title,
                "description": args.description,
                "priority": args.priority,
            }
        },
    )
    payload = data.get("issueCreate") or {}
    if args.json:
        return print_json(payload)
    issue = payload.get("issue") or {}
    print(f"success={payload.get('success', False)}")
    print(f"{issue.get('identifier', '?')}  {issue.get('title', '?')}")
    if issue.get("url"):
        print(issue["url"])
    return 0


def update_issue(args: argparse.Namespace, input_payload: dict[str, Any], fields: str) -> int:
    data = post_graphql(
        f"""
        mutation($issueId: String!, $input: IssueUpdateInput!) {{
          issueUpdate(id: $issueId, input: $input) {{
            success
            issue {{ identifier {fields} }}
          }}
        }}
        """,
        variables={"issueId": args.issue_id, "input": input_payload},
    )
    payload = data.get("issueUpdate") or {}
    if args.json:
        return print_json(payload)
    print(json.dumps(payload, indent=2))
    return 0


def cmd_update_state(args: argparse.Namespace) -> int:
    return update_issue(args, {"stateId": args.state_id}, "state { id name type }")


def cmd_assign(args: argparse.Namespace) -> int:
    return update_issue(args, {"assigneeId": args.assignee_id}, "assignee { id name email }")


def cmd_set_priority(args: argparse.Namespace) -> int:
    return update_issue(args, {"priority": args.priority}, "priority")


def cmd_add_comment(args: argparse.Namespace) -> int:
    data = post_graphql(
        """
        mutation($input: CommentCreateInput!) {
          commentCreate(input: $input) {
            success
            comment { id body }
          }
        }
        """,
        variables={"input": {"issueId": args.issue_id, "body": args.body}},
    )
    payload = data.get("commentCreate") or {}
    if args.json:
        return print_json(payload)
    print(json.dumps(payload, indent=2))
    return 0


def cmd_request(args: argparse.Namespace) -> int:
    variables = json.loads(args.variables_json) if args.variables_json else None
    data = post_graphql(args.query, variables=variables)
    return print_json(data)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Linear API helper")
    subparsers = parser.add_subparsers(dest="command", required=True)

    viewer = subparsers.add_parser("viewer", help="Show current authenticated user")
    viewer.add_argument("--json", action="store_true")
    viewer.set_defaults(func=cmd_viewer)

    teams = subparsers.add_parser("teams", help="List teams")
    teams.add_argument("--json", action="store_true")
    teams.set_defaults(func=cmd_teams)

    states = subparsers.add_parser("workflow-states", help="List workflow states for a team")
    states.add_argument("--team-key", required=True)
    states.add_argument("--json", action="store_true")
    states.set_defaults(func=cmd_workflow_states)

    issues = subparsers.add_parser("issues", help="List issues")
    issues.add_argument("--limit", type=int, default=20)
    issues.add_argument("--team-key")
    issues.add_argument("--assignee-email")
    issues.add_argument("--state-type", choices=["triage", "backlog", "unstarted", "started", "completed", "canceled"])
    issues.add_argument("--json", action="store_true")
    issues.set_defaults(func=cmd_issues)

    issue = subparsers.add_parser("issue", help="Show one issue by ID or identifier")
    issue.add_argument("--issue-id", required=True)
    issue.add_argument("--json", action="store_true")
    issue.set_defaults(func=cmd_issue)

    issue_search = subparsers.add_parser("issue-search", help="Search issues by text")
    issue_search.add_argument("--query", required=True)
    issue_search.add_argument("--limit", type=int, default=10)
    issue_search.add_argument("--json", action="store_true")
    issue_search.set_defaults(func=cmd_issue_search)

    projects = subparsers.add_parser("projects", help="List projects")
    projects.add_argument("--limit", type=int, default=20)
    projects.add_argument("--json", action="store_true")
    projects.set_defaults(func=cmd_projects)

    users = subparsers.add_parser("users", help="List users")
    users.add_argument("--json", action="store_true")
    users.set_defaults(func=cmd_users)

    labels = subparsers.add_parser("labels", help="List issue labels")
    labels.add_argument("--json", action="store_true")
    labels.set_defaults(func=cmd_labels)

    create_issue = subparsers.add_parser("create-issue", help="Create an issue")
    create_issue.add_argument("--team-id", required=True)
    create_issue.add_argument("--title", required=True)
    create_issue.add_argument("--description", default="")
    create_issue.add_argument("--priority", type=int, choices=[0, 1, 2, 3, 4], default=0)
    create_issue.add_argument("--json", action="store_true")
    create_issue.set_defaults(func=cmd_create_issue)

    update_state = subparsers.add_parser("update-state", help="Set an issue workflow state")
    update_state.add_argument("--issue-id", required=True)
    update_state.add_argument("--state-id", required=True)
    update_state.add_argument("--json", action="store_true")
    update_state.set_defaults(func=cmd_update_state)

    assign = subparsers.add_parser("assign", help="Assign an issue to a user")
    assign.add_argument("--issue-id", required=True)
    assign.add_argument("--assignee-id", required=True)
    assign.add_argument("--json", action="store_true")
    assign.set_defaults(func=cmd_assign)

    set_priority = subparsers.add_parser("set-priority", help="Set issue priority")
    set_priority.add_argument("--issue-id", required=True)
    set_priority.add_argument("--priority", type=int, choices=[0, 1, 2, 3, 4], required=True)
    set_priority.add_argument("--json", action="store_true")
    set_priority.set_defaults(func=cmd_set_priority)

    add_comment = subparsers.add_parser("add-comment", help="Add a comment to an issue")
    add_comment.add_argument("--issue-id", required=True)
    add_comment.add_argument("--body", required=True)
    add_comment.add_argument("--json", action="store_true")
    add_comment.set_defaults(func=cmd_add_comment)

    request = subparsers.add_parser("request", help="Send a raw GraphQL query")
    request.add_argument("--query", required=True)
    request.add_argument("--variables-json")
    request.set_defaults(func=cmd_request)

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
