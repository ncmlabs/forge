---
name: slack
description: Slack operations via the Web API — messages, channels, threads, reactions, and approvals.
timeout: 120
allowed-tools:
  - Bash(curl:*)
capabilities:
  - name: send_message
    inputs: [Text, Text]
    output: Text
    params: [channel, text]
    executor:
      kind: command
      argv: [curl, -s, -X, POST, "https://slack.com/api/chat.postMessage", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "text={text}"]
      result:
        success_path: ok
        error_path: error
        json_path: ts
  - name: send_rich_message
    inputs: [Text, Text]
    output: Text
  - name: reply_thread
    inputs: [Text, Text, Text]
    output: Text
    params: [channel, thread_ts, text]
    executor:
      kind: command
      argv: [curl, -s, -X, POST, "https://slack.com/api/chat.postMessage", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "thread_ts={thread_ts}", --data-urlencode, "text={text}"]
      result:
        success_path: ok
        error_path: error
        json_path: ts
  - name: add_reaction
    inputs: [Text, Text, Text]
    output: Text
    params: [channel, timestamp, emoji]
    executor:
      kind: command
      argv: [curl, -s, -X, POST, "https://slack.com/api/reactions.add", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "timestamp={timestamp}", --data-urlencode, "name={emoji}"]
      result:
        success_path: ok
        error_path: error
  - name: list_channels
    inputs: [Text]
    output: Text
  - name: read_history
    inputs: [Text, Text, Text]
    output: Text
    params: [channel, oldest, limit]
    executor:
      kind: command
      argv: [curl, -s, -G, "https://slack.com/api/conversations.history", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "oldest={oldest}", --data-urlencode, "limit={limit}"]
      result:
        success_path: ok
        error_path: error
        json_path: messages
  - name: read_thread
    inputs: [Text, Text]
    output: Text
    params: [channel, thread_ts]
    executor:
      kind: command
      argv: [curl, -s, -G, "https://slack.com/api/conversations.replies", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "ts={thread_ts}"]
      result:
        success_path: ok
        error_path: error
        json_path: messages
  - name: detect_mentions
    inputs: [Text, Text]
    output: Text
  - name: send_approval
    inputs: [Text, Text, Text, Text]
    output: Text
  - name: edit_message
    inputs: [Text, Text, Text]
    output: Text
    params: [channel, timestamp, text]
    executor:
      kind: command
      argv: [curl, -s, -X, POST, "https://slack.com/api/chat.update", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "ts={timestamp}", --data-urlencode, "text={text}"]
      result:
        success_path: ok
        error_path: error
  - name: delete_message
    inputs: [Text, Text]
    output: Text
    params: [channel, timestamp]
    executor:
      kind: command
      argv: [curl, -s, -X, POST, "https://slack.com/api/chat.delete", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "ts={timestamp}"]
      result:
        success_path: ok
        error_path: error
  - name: pin_message
    inputs: [Text, Text]
    output: Text
    params: [channel, timestamp]
    executor:
      kind: command
      argv: [curl, -s, -X, POST, "https://slack.com/api/pins.add", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "channel={channel}", --data-urlencode, "timestamp={timestamp}"]
      result:
        success_path: ok
        error_path: error
  - name: member_info
    inputs: [Text]
    output: Text
    params: [user_id]
    executor:
      kind: command
      argv: [curl, -s, -G, "https://slack.com/api/users.info", -H, "Authorization: Bearer {env:SLACK_BOT_TOKEN}", --data-urlencode, "user={user_id}"]
      result:
        success_path: ok
        error_path: error
        json_path: user
---

# Slack Skill

Use this skill to interact with Slack workspaces via the Web API and `curl`.
Only run commands that start with `curl` targeting `https://slack.com/api/`. Do not run arbitrary shell commands.

## Prerequisites

The `SLACK_BOT_TOKEN` environment variable must be set to a valid bot token (`xoxb-...`).

Before any operation, verify the token:

```bash
curl -s "https://slack.com/api/auth.test" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN"
```

If the response contains `"ok": false`, return `"ERROR: Slack auth failed. Check SLACK_BOT_TOKEN."` and stop.

### Required Bot Token Scopes

| Scope | Used by |
|---|---|
| `chat:write` | send_message, send_rich_message, reply_thread, edit_message, delete_message |
| `channels:history` | read_history, read_thread, detect_mentions (public channels) |
| `groups:history` | read_history, read_thread, detect_mentions (private channels) |
| `im:history` | read_history (DMs) |
| `mpim:history` | read_history (group DMs) |
| `channels:read` | list_channels (public) |
| `groups:read` | list_channels (private) |
| `im:read` | list_channels (DMs) |
| `mpim:read` | list_channels (group DMs) |
| `reactions:write` | add_reaction |
| `pins:write` | pin_message |
| `users:read` | member_info |

### Bot Token Setup

1. Go to https://api.slack.com/apps and click **Create New App** → **From scratch**
2. Under **OAuth & Permissions**, add the scopes listed above to **Bot Token Scopes**
3. Click **Install to Workspace** and authorize
4. Copy the **Bot User OAuth Token** (`xoxb-...`) and set it as `SLACK_BOT_TOKEN`
5. Invite the bot to channels it needs to access: `/invite @YourBotName`

## Capabilities

### `send_message(channel, text)`

Send a plain-text message to a channel or DM.

```bash
curl -s -X POST "https://slack.com/api/chat.postMessage" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "text": "Hello from FORGE"
  }'
```

Return the `ts` (message timestamp) from the response on success. This `ts` is the message ID used by other capabilities.

### `send_rich_message(channel, blocks_json)`

Send a message with Block Kit layout blocks for rich formatting.

```bash
curl -s -X POST "https://slack.com/api/chat.postMessage" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "text": "Fallback text for notifications",
    "blocks": [
      {
        "type": "header",
        "text": {"type": "plain_text", "text": "Deploy Report"}
      },
      {
        "type": "section",
        "text": {"type": "mrkdwn", "text": "*Status:* All checks passed\n*Branch:* `main`\n*Commit:* `abc1234`"}
      },
      {
        "type": "divider"
      },
      {
        "type": "section",
        "text": {"type": "mrkdwn", "text": "View the full report:"},
        "accessory": {
          "type": "button",
          "text": {"type": "plain_text", "text": "Open Dashboard"},
          "url": "https://example.com/dashboard"
        }
      }
    ]
  }'
```

Always include a `text` field as fallback for notifications and accessibility. The `blocks` array accepts any valid Block Kit layout block — see https://api.slack.com/reference/block-kit/blocks.

Return the `ts` on success.

### `reply_thread(channel, thread_ts, text)`

Reply to a message in a thread. The `thread_ts` is the `ts` of the parent message.

```bash
curl -s -X POST "https://slack.com/api/chat.postMessage" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "thread_ts": "1712023032.123400",
    "text": "Thread reply from FORGE"
  }'
```

To also broadcast the reply to the channel, add `"reply_broadcast": true`. Use sparingly — only when everyone needs visibility.

Return the `ts` on success.

### `add_reaction(channel, timestamp, emoji)`

Add an emoji reaction to a message. Use the emoji name without colons.

```bash
curl -s -X POST "https://slack.com/api/reactions.add" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "timestamp": "1712023032.123400",
    "name": "white_check_mark"
  }'
```

Common emoji names: `white_check_mark`, `eyes`, `thumbsup`, `thumbsdown`, `rocket`, `warning`, `x`.

Return confirmation on success.

### `list_channels(filter)`

List channels the bot has access to. The `filter` parameter sets the channel types to include.

```bash
curl -s -G "https://slack.com/api/conversations.list" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  --data-urlencode "types=public_channel,private_channel" \
  --data-urlencode "exclude_archived=true" \
  --data-urlencode "limit=200"
```

Valid types: `public_channel`, `private_channel`, `mpim` (group DMs), `im` (DMs).

The response includes a `channels` array with `id`, `name`, `purpose`, `topic`, and `num_members`.

For pagination, if `response_metadata.next_cursor` is non-empty, pass it as `cursor` in the next request:

```bash
curl -s -G "https://slack.com/api/conversations.list" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  --data-urlencode "types=public_channel" \
  --data-urlencode "cursor=dXNlcjpVMDYxTkZUVDI=" \
  --data-urlencode "limit=200"
```

Return the JSON array of channels.

### `read_history(channel, oldest, limit)`

Read messages from a channel. Use `oldest` (Unix timestamp) for incremental polling — only messages after that timestamp are returned.

```bash
curl -s -G "https://slack.com/api/conversations.history" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  --data-urlencode "channel=C0123456789" \
  --data-urlencode "oldest=1712023032.000000" \
  --data-urlencode "limit=50"
```

Omit `oldest` to get the most recent messages. The response `messages` array is in reverse chronological order (newest first). Each message has `ts`, `user`, `text`, and optionally `thread_ts` (if it is a thread parent or reply).

For pagination, check `has_more` and use `response_metadata.next_cursor`.

Return the JSON messages array.

### `read_thread(channel, thread_ts)`

Read all replies in a thread. The `thread_ts` is the parent message timestamp.

```bash
curl -s -G "https://slack.com/api/conversations.replies" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  --data-urlencode "channel=C0123456789" \
  --data-urlencode "ts=1712023032.123400"
```

The first message in the response is the thread parent. Subsequent messages are replies in chronological order.

Return the JSON messages array.

### `detect_mentions(channel, oldest)`

Read channel history and filter for messages that @mention the bot. This is a composite operation.

Step 1 — Get the bot user ID:

```bash
curl -s "https://slack.com/api/auth.test" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" | jq -r '.user_id'
```

Step 2 — Read history and filter for mentions:

```bash
curl -s -G "https://slack.com/api/conversations.history" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  --data-urlencode "channel=C0123456789" \
  --data-urlencode "oldest=1712023032.000000" \
  --data-urlencode "limit=100" \
  | jq '[.messages[] | select(.text | contains("<@BOT_USER_ID>"))]'
```

Replace `BOT_USER_ID` with the value from Step 1. Store the latest `ts` from the results to use as `oldest` in the next poll cycle.

Return the filtered messages array.

### `send_approval(channel, text, callback_url, request_id)`

Send a message with interactive Approve/Reject buttons. The `request_id` is encoded in each button's `value` field as `"approved:{request_id}"` or `"rejected:{request_id}"` so the receiving webhook can correlate responses to the original approval request.

```bash
curl -s -X POST "https://slack.com/api/chat.postMessage" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "text": "Approval required: Deploy v2.1.0 to production",
    "blocks": [
      {
        "type": "section",
        "text": {"type": "mrkdwn", "text": "*Approval Required*\nDeploy v2.1.0 to production"}
      },
      {
        "type": "actions",
        "block_id": "approval_actions",
        "elements": [
          {
            "type": "button",
            "text": {"type": "plain_text", "text": "Approve"},
            "style": "primary",
            "action_id": "approve",
            "value": "approved:REQUEST_ID"
          },
          {
            "type": "button",
            "text": {"type": "plain_text", "text": "Reject"},
            "style": "danger",
            "action_id": "reject",
            "value": "rejected:REQUEST_ID"
          }
        ]
      }
    ]
  }'
```

Replace `REQUEST_ID` with the `request_id` parameter value.

To receive button clicks, configure an **Interactivity Request URL** in your Slack app settings (https://api.slack.com/apps → your app → **Interactivity & Shortcuts** → set the Request URL to your `callback_url`). The FORGE server exposes `POST /webhook/approval` as the dedicated endpoint for this purpose.

When a user clicks a button, Slack POSTs a form-encoded payload to the callback URL. The FORGE `/webhook/approval` endpoint parses the `actions[0].value` field to extract the decision and `request_id`, then publishes an `ApprovalResponse` event to the event bus with fields: `request_id` (Text), `approved` (Bool), `comment` (Text with approver info).

Return the `ts` on success.

### `edit_message(channel, timestamp, text)`

Update an existing message. The bot can only edit messages it posted.

```bash
curl -s -X POST "https://slack.com/api/chat.update" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "ts": "1712023032.123400",
    "text": "Updated message text"
  }'
```

Return confirmation on success.

### `delete_message(channel, timestamp)`

Delete a message. The bot can only delete messages it posted (unless it has `chat:write.public` scope).

```bash
curl -s -X POST "https://slack.com/api/chat.delete" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "ts": "1712023032.123400"
  }'
```

Return confirmation on success.

### `pin_message(channel, timestamp)`

Pin a message to a channel.

```bash
curl -s -X POST "https://slack.com/api/pins.add" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "C0123456789",
    "timestamp": "1712023032.123400"
  }'
```

Return confirmation on success.

### `member_info(user_id)`

Get profile information for a Slack user.

```bash
curl -s -G "https://slack.com/api/users.info" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  --data-urlencode "user=U0123456789"
```

The response `user` object includes `real_name`, `display_name`, `email` (if `users:read.email` scope is added), `title`, `status_text`, and `tz`.

Return the user profile JSON.

## Rate Limit Guidance

Slack uses tiered rate limits. The relevant tiers for this skill:

| Tier | Limit | Methods |
|---|---|---|
| Tier 2 | ~20 req/min | `chat.postMessage`, `chat.update`, `chat.delete`, `reactions.add`, `pins.add` |
| Tier 3 | ~50 req/min | `conversations.history`, `conversations.list`, `conversations.replies` |
| Tier 4 | ~100 req/min | `users.info`, `auth.test` |

When rate-limited, the API returns HTTP 429 with a `Retry-After` header (seconds to wait). Respect it:

```bash
# Check for rate limiting in response headers
curl -s -D - -G "https://slack.com/api/conversations.history" \
  -H "Authorization: Bearer $SLACK_BOT_TOKEN" \
  --data-urlencode "channel=C0123456789" \
  --data-urlencode "limit=50" \
  | head -20
```

If you see `HTTP/2 429`, wait the number of seconds in `Retry-After` before retrying.

For bulk operations (reading many channels), add 1-second delays between requests to stay well under limits.

## Error Handling

### Authentication failures

If any request returns `"error": "not_authed"` or `"error": "invalid_auth"`:

Return `"ERROR: Slack auth failed. Verify SLACK_BOT_TOKEN is set and valid."` — do not retry.

### Missing scopes

If a request returns `"error": "missing_scope"` with a `"needed"` field:

Return `"ERROR: Missing Slack scope '<needed>'. Add it at https://api.slack.com/apps under OAuth & Permissions."` — do not retry.

### Channel not found

If a request returns `"error": "channel_not_found"`:

Return `"ERROR: Channel not found. Verify the channel ID and that the bot is a member."` — do not guess alternatives.

### Bot not in channel

If a request returns `"error": "not_in_channel"`:

Return `"ERROR: Bot is not in this channel. Invite it with /invite @BotName in Slack."` — do not retry.

### Rate limited

If a request returns HTTP 429 or `"error": "ratelimited"`:

Return `"ERROR: Rate limited. Retry after <Retry-After> seconds."` — wait before retrying.

### Message not found

If `chat.update` or `chat.delete` returns `"error": "message_not_found"`:

Return `"ERROR: Message not found. Verify the channel and timestamp."` — do not retry.

## Safety Rules

- Only run `curl` commands targeting `https://slack.com/api/*`. No other URLs.
- Never log, echo, or print `$SLACK_BOT_TOKEN` in output.
- Do not run arbitrary shell commands beyond `curl` and `jq` for parsing.
- Return the curl command that was run alongside the result for auditability (with the token replaced by `$SLACK_BOT_TOKEN`).
- Do not delete messages you did not post unless explicitly instructed.
- Do not store or cache message content outside the current session.

## Session Adapter Mapping

| FORGE session field | curl mapping |
|---|---|
| `channel` | `"channel"` in JSON body or `channel` query param |
| `token` | `Authorization: Bearer $SLACK_BOT_TOKEN` header |
| `output_mode = json` | All Slack API responses are JSON by default |
| `timeout` | Managed by FORGE skill executor |

## AgentResult Mapping

| Slack API response | AgentResult field |
|---|---|
| `ts` (message timestamp) | `output` |
| JSON array (messages, channels) | `output` (structured) |
| `"ok": false` with `"error"` | `output` (prefixed with `ERROR:`) |
| curl command executed | `metadata.command` |
