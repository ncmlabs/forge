import { test, expect } from '@playwright/test';
import { TopologyPage } from '../pages/sentinel.page';

// These tests require a real ANTHROPIC_API_KEY and hit the actual Claude API.
// They verify the full live topology experience: scan triggers LLM calls,
// SSE events stream in real-time, nodes pulse, edges animate, and detail
// panels populate with real agent data.
// Explicit opt-in only (#288): both FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY
// must be set — otherwise the tests skip cleanly so a default run never
// makes paid calls.
test.beforeEach(async () => {
  test.skip(
    process.env.FORGE_LLM_LIVE !== '1' || !process.env.ANTHROPIC_API_KEY,
    'requires FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY'
  );
});

test.describe('Sentinel Topology — Real LLM Live Graph (issue #141)', () => {
  test('topology renders live agents from real system runtime', async ({ page }) => {
    test.setTimeout(30_000);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    // Real system runtime should produce real nodes
    expect(await topo.nodeCount('system')).toBe(1);
    expect(await topo.nodeCount('warden')).toBe(1);
    expect(await topo.nodeCount('agent')).toBeGreaterThanOrEqual(2);
  });

  test('SSE connects and streams events from live runtime', async ({ page }) => {
    test.setTimeout(30_000);
    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectSSEConnected();
    await expect(topo.sseStatus).toHaveText('Live');
  });

  test('scan triggers real LLM calls and SSE events appear', async ({ page }) => {
    test.setTimeout(120_000); // Scan involves exec, reason, classify, pool — multiple LLM calls

    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();
    await topo.expectSSEConnected();

    // Trigger scan
    await topo.scanButton.click();
    await expect(topo.scanButton).toBeDisabled();
    await expect(topo.scanButton).toContainText('Scanning...');

    // Wait for SSE events to flow — at minimum: http_request, task_call, exec_call, llm_request
    await topo.expectMinEvents(4, 90_000);

    // Scan should complete and re-enable button
    await expect(topo.scanButton).toBeEnabled({ timeout: 90_000 });
    await expect(topo.scanButton).toContainText('Run Scan');
  });

  test('event log shows real trace events during scan', async ({ page }) => {
    test.setTimeout(120_000);

    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();
    await topo.expectSSEConnected();

    await topo.scanButton.click();

    // Wait for substantial event flow
    await topo.expectMinEvents(6, 90_000);

    // Verify event log contains real event types
    const logText = await topo.eventLog.textContent();
    // A real scan produces exec (git commands), reason (LLM analysis), and likely flow events
    const hasExecOrTask = logText!.includes('exec') || logText!.includes('task');
    expect(hasExecOrTask).toBe(true);
  });

  test('agent detail shows real memory after scan', async ({ page }) => {
    test.setTimeout(120_000);

    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();
    await topo.expectSSEConnected();

    // Trigger scan and wait for completion
    await topo.scanButton.click();
    await expect(topo.scanButton).toBeEnabled({ timeout: 90_000 });

    // Click the inspector agent — should have real memory from scan
    await topo.clickNode('agent:git_inspector');
    await topo.expectDetailPanel();

    const text = await topo.detailContent.textContent();
    expect(text).toContain('git_inspector');
    expect(text).toContain('Lifecycle');
    expect(text).toContain('Uptime');
    // Real scan should populate memory fields
    expect(text).toContain('Memory');
  });

  test('warden detail shows real supervision state', async ({ page }) => {
    test.setTimeout(30_000);

    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    await topo.clickNode('warden:sentinel_supervisor');
    await topo.expectDetailPanel();

    const text = await topo.detailContent.textContent();
    expect(text).toContain('sentinel_supervisor');
    expect(text).toContain('Managed agents');
    // At least one managed agent should be listed (timing may affect which are live)
    const hasAgent = text!.includes('git_inspector') || text!.includes('analyst');
    expect(hasAgent).toBe(true);
    expect(text).toContain('Circuit breaker');
  });

  test('edges exist between real agents and wardens', async ({ page }) => {
    test.setTimeout(30_000);

    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();

    const types = await topo.linkTypes();
    // System -> warden, warden -> agents = supervision edges
    expect(types.filter((t) => t === 'supervises').length).toBeGreaterThanOrEqual(3);
    // inspector >> analyst = wired edge
    expect(types).toContain('wired');
  });

  test('multiple scans work without page reload', async ({ page }) => {
    test.setTimeout(240_000);

    const topo = new TopologyPage(page);
    await topo.goto();
    await topo.expectGraphLoaded();
    await topo.expectSSEConnected();

    // First scan
    await topo.scanButton.click();
    await expect(topo.scanButton).toBeEnabled({ timeout: 120_000 });

    // Second scan — graph should remain stable
    await topo.scanButton.click();
    await expect(topo.scanButton).toBeEnabled({ timeout: 120_000 });

    // Graph should still have all nodes after two scans
    expect(await topo.nodeCount('system')).toBe(1);
    expect(await topo.nodeCount('warden')).toBe(1);
    expect(await topo.nodeCount('agent')).toBeGreaterThanOrEqual(2);

    // Event log should have entries from the scans
    const finalCount = await topo.eventLogCount();
    expect(finalCount).toBeGreaterThan(0);
  });
});
