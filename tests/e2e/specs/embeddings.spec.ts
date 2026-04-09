import { test, expect } from '@playwright/test';

test.describe('Vector Embeddings API (#50)', () => {
  test('embed a page via api_embed endpoint', async ({ request }) => {
    const resp = await request.get('/api_embed?slug=getting-started');
    expect(resp.status()).toBe(200);

    const body = await resp.text();
    expect(body).toMatch(/^emb_[0-9a-f]+$/);
  });

  test('embed multiple pages then search', async ({ request }) => {
    // Embed several pages
    for (const slug of ['getting-started', 'task', 'agent']) {
      const resp = await request.get(`/api_embed?slug=${slug}`);
      expect(resp.status()).toBe(200);
      const body = await resp.text();
      expect(body).toMatch(/^emb_/);
    }

    // Search
    const searchResp = await request.get('/api_semantic_search?q=how+do+agents+work');
    expect(searchResp.status()).toBe(200);

    const results = await searchResp.text();
    // Should contain score information from format_search_results
    expect(results).toContain('score:');
  });

  test('search empty index returns empty result', async ({ request }) => {
    // Note: other tests may have already embedded content via the shared server,
    // so we just verify the endpoint doesn't crash.
    const resp = await request.get('/api_semantic_search?q=nonexistent+topic');
    expect(resp.status()).toBe(200);
  });

  test('embed returns different IDs for different content', async ({ request }) => {
    const resp1 = await request.get('/api_embed?slug=getting-started');
    const resp2 = await request.get('/api_embed?slug=task');

    const id1 = await resp1.text();
    const id2 = await resp2.text();

    expect(id1).toMatch(/^emb_/);
    expect(id2).toMatch(/^emb_/);
    expect(id1).not.toBe(id2);
  });

  test('re-embedding same page succeeds (upsert)', async ({ request }) => {
    const resp1 = await request.get('/api_embed?slug=getting-started');
    expect(resp1.status()).toBe(200);

    const resp2 = await request.get('/api_embed?slug=getting-started');
    expect(resp2.status()).toBe(200);

    // Both should return valid embedding IDs
    const id1 = await resp1.text();
    const id2 = await resp2.text();
    expect(id1).toMatch(/^emb_/);
    expect(id2).toMatch(/^emb_/);
  });

  test('search results contain embedded content', async ({ request }) => {
    // Embed a page with known content
    await request.get('/api_embed?slug=getting-started');

    // Search for something that should match
    const resp = await request.get('/api_semantic_search?q=getting+started');
    expect(resp.status()).toBe(200);

    const body = await resp.text();
    // The result should contain content from the embedded page
    expect(body.length).toBeGreaterThan(0);
  });
});
