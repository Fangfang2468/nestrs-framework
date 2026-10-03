#!/usr/bin/env node
"use strict";

// Uses an existing Playwright installation; this repository needs no npm manifest.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { pathToFileURL } = require("node:url");

const HELP = `Verify the offline dependency graph page using Chromium and Playwright.

Usage: node tools/verify-graph-page.cjs --html PATH [OPTIONS]

  --html PATH        Exported checkout v1 graph or v2 project report (required)
  --output PATH      Reports, fixture pages and screenshots (default: target/nestrs-graph-page)
  --template PATH    Test a newer HTML template with the exported data, without rebuilding
  --playwright PATH  External Playwright package directory or entry point
  --chromium PATH    Chromium executable; defaults to Playwright's installed browser
  --help             Show this help

NESTRS_PLAYWRIGHT_MODULE and NESTRS_CHROMIUM_PATH also configure external tools.
The input must include the real checkout example graph. Synthetic fixtures use
the same HTML to test repeated slots, entry isolation, keys and untrusted text.
No Cargo command, package installation or network request is needed.
`;

function options(argv) {
  const result = { output: "target/nestrs-graph-page" };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    if (key === "--help" || key === "-h") {
      process.stdout.write(HELP);
      process.exit(0);
    }
    assert(["--html", "--output", "--template", "--playwright", "--chromium"].includes(key), `Unknown argument: ${key}`);
    const value = argv[++i];
    assert(value && !value.startsWith("--"), `${key} requires a path`);
    result[key.slice(2)] = value;
  }
  assert(result.html, "--html is required; see --help");
  return result;
}

function readGraph(html) {
  const matches = [...html.matchAll(/<script\b[^>]*\bid=["']graph-data["'][^>]*>([\s\S]*?)<\/script>/gi)];
  assert.equal(matches.length, 1, "Expected exactly one embedded graph-data script");
  return JSON.parse(matches[0][1]);
}

function embedGraph(html, graph) {
  const json = JSON.stringify(graph).replace(/[<>&\u2028\u2029]/g, (character) => ({
    "<": "\\u003c", ">": "\\u003e", "&": "\\u0026", "\u2028": "\\u2028", "\u2029": "\\u2029",
  })[character]);
  let replacements = 0;
  const result = html.replace(/(<script\b[^>]*\bid=["']graph-data["'][^>]*>)[\s\S]*?(<\/script>)/gi, (_, before, after) => {
    replacements += 1;
    return before + json + after;
  });
  assert.equal(replacements, 1, "HTML template must contain one graph-data script");
  return result;
}

const ATTACK = '</script><img src="https://invalid.test/graph" onerror="globalThis.__nestrsGraphInjected=true">';
const NAMED = (value) => ({ kind: "named", value });
function syntheticGraph() {
  const provider = (id, name, lifetime = "Singleton") => ({
    id, name: `fixture::${name}`, label: name, lifetime, kind: "class", key: null,
    primary: false, requiresScope: lifetime === "Scoped", depth: 0,
    source: { file: `src/services.rs ${ATTACK}`, line: id + 1, column: 1 }, dependencies: [],
  });
  const nodes = [provider(0, "Consumer", "Scoped"), provider(1, `OtherConsumer${ATTACK}`, "Scoped"),
    provider(2, "CardImplementation"), provider(3, "Formatter", "Transient"), provider(4, "WalletImplementation")];
  nodes[2].key = NAMED("card");
  nodes[4].key = NAMED("wallet");
  const dependency = (slot, label, target, requested, key = null, optional = false) => ({ slot, label, target, requested, requestedLabel: requested.replace("fixture::", ""), key, optional });
  nodes[0].dependencies = [
    dependency(0, "receipt", 3, nodes[3].name), dependency(1, "copy", 3, nodes[3].name),
    dependency(2, "fraud", null, "dyn fixture::Missing", null, true),
    dependency(3, "card", 2, "dyn fixture::Port", NAMED("card")),
    dependency(4, "wallet", 4, "dyn fixture::Port", NAMED("wallet")),
  ];
  nodes[1].dependencies = [dependency(0, `shared-card ${ATTACK}`, 2, "dyn fixture::Port", NAMED("card"))];
  return { version: 1, nodes };
}

function largeGraph() {
  const base = syntheticGraph();
  const consumer = { ...base.nodes[0], name: "fixture::LargeConsumer", label: "LargeConsumer" };
  const formatter = { ...base.nodes[3], id: 1 };
  consumer.dependencies = Array.from({ length: 300 }, (_, slot) => {
    const optional = slot >= 220;
    const requested = slot < 2 ? formatter.name : optional ? `dyn fixture::${slot === 299 ? "LateOptional" : `Missing${slot}`}` : `dyn fixture::${slot === 219 ? "LateInterface" : "RepeatedPort"}`;
    return {
      slot, label: optional ? `optional_${slot}` : `input_${slot}`, target: optional ? null : 1,
      requested, requestedLabel: requested.replace("fixture::", ""), key: null, optional,
    };
  });
  return { version: 1, nodes: [consumer, formatter] };
}

function projectGraph(graph, includeValid = true) {
  const entry = (id, binary, status, value, diagnostic = null) => ({
    id, packageId: "fixture-package", package: `fixture-package ${ATTACK}`, binary,
    status, graph: value, diagnostic, requiredFeatures: status === "skipped" ? [`extra ${ATTACK}`] : [],
  });
  return {
    version: 2, kind: "project", packages: [{ id: "fixture-package", name: `fixture-package ${ATTACK}`, version: "0.0.0" }],
    entries: [
      ...(includeValid ? [entry("entry-a", "first", "ok", graph), entry("entry-b", "second", "ok", structuredClone(graph))] : []),
      entry("entry-error", `broken ${ATTACK}`, "error", null, `graph validation failed ${ATTACK}`),
      entry("entry-skipped", "feature-gated", "skipped", null, `disabled required features ${ATTACK}`),
    ],
  };
}

function keyParts(key) {
  return key === null ? ["none", ""] : [key.kind, String(key.value)];
}

function expectedId(id, entryId) {
  return entryId === undefined ? String(id) : JSON.stringify([entryId, id]);
}

async function state(page) {
  return page.evaluate(() => ({
    nodes: [...document.querySelectorAll("#graph-nodes .node")].map((node) => ({
      id: node.getAttribute("data-node-id"), kind: node.getAttribute("data-node-kind"),
      entryId: node.getAttribute("data-entry-id"), target: node.getAttribute("data-projection-target"),
      requested: node.getAttribute("data-requested"), keyKind: node.getAttribute("data-key-kind"),
      keyValue: node.getAttribute("data-key-value"), initialization: node.getAttribute("data-initialization"), text: node.textContent,
    })),
    edges: [...document.querySelectorAll("#graph-edges .edge")].map((edge) => ({
      source: edge.getAttribute("data-edge-source"), target: edge.getAttribute("data-edge-target"),
      kind: edge.getAttribute("data-edge-kind"), slot: edge.getAttribute("data-slot"),
      label: edge.getAttribute("data-input-label"), requested: edge.getAttribute("data-requested"),
      keyKind: edge.getAttribute("data-key-kind"), keyValue: edge.getAttribute("data-key-value"),
      lazy: edge.getAttribute("data-lazy"),
    })),
    labels: [...document.querySelectorAll("#graph-edges .edge-label")].map((label) => label.textContent),
  }));
}

async function selectNode(page, id) {
  const selector = `.node[data-node-id=${JSON.stringify(id)}]`;
  await page.locator(selector).focus();
  await page.keyboard.press("Enter");
  await page.waitForFunction((value) => document.querySelector(".node.is-selected")?.getAttribute("data-node-id") === value, id);
}

async function assertCounts(page, providers, projections, optional) {
  assert.equal((await page.locator("#provider-count").innerText()).replaceAll(",", ""), String(providers));
  assert.equal((await page.locator("#projection-count").innerText()).replaceAll(",", ""), String(projections));
  assert.equal((await page.locator("#optional-count").innerText()).replaceAll(",", ""), String(optional));
}

async function assertGeometry(page) {
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const problems = await page.evaluate(() => {
    const nodes = [...document.querySelectorAll("#graph-nodes .node")].map((node) => ({
      id: node.getAttribute("data-node-id"), frame: node.querySelector(".node-frame").getBoundingClientRect(),
    }));
    const issues = [];
    const intersects = (a, b, inset = 1) => Math.min(a.right, b.right) - Math.max(a.left, b.left) > inset && Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > inset;
    for (const edge of document.querySelectorAll("#graph-edges .edge")) {
      const from = edge.getAttribute("data-edge-source");
      const to = edge.getAttribute("data-edge-target");
      const matrix = edge.getScreenCTM();
      const length = edge.getTotalLength();
      for (const node of nodes) {
        if (node.id === from || node.id === to) continue;
        const box = node.frame;
        for (let distance = 0; distance <= length; distance += 3) {
          const local = edge.getPointAtLength(distance);
          const point = new DOMPoint(local.x, local.y).matrixTransform(matrix);
          if (point.x > box.left + 2 && point.x < box.right - 2 && point.y > box.top + 2 && point.y < box.bottom - 2) {
            issues.push(`Edge ${from} -> ${to} crosses unrelated node ${node.id}`);
            break;
          }
        }
      }
    }
    const labels = [...document.querySelectorAll("#graph-edges .edge-label")].map((label) => ({ text: label.textContent, box: label.getBoundingClientRect() }));
    for (let index = 0; index < labels.length; index += 1) {
      const label = labels[index];
      for (const node of nodes) if (intersects(label.box, node.frame)) issues.push(`Edge label ${label.text} overlaps node ${node.id}`);
      for (const other of labels.slice(index + 1)) if (intersects(label.box, other.box)) issues.push(`Edge labels overlap: ${label.text} / ${other.text}`);
    }
    return issues;
  });
  assert.deepEqual(problems, [], "Dependency lines and their labels must remain distinct from unrelated nodes");
}

async function verifyVisibleGraph(page, graph, entryId) {
  const snapshot = await state(page);
  const own = (node) => node.entryId === (entryId ?? "");
  const projections = snapshot.nodes.filter((node) => node.kind === "projection" && own(node));
  const providers = snapshot.nodes.filter((node) => node.kind === "provider" && own(node));
  assert.equal(providers.length, graph.nodes.length, "Interface nodes must not replace providers");
  const requests = new Set();
  let inputCount = 0;
  for (const consumer of graph.nodes) {
    const source = expectedId(consumer.id, entryId);
    assert(providers.some((node) => node.id === source), `Missing provider ${source}`);
    for (const dependency of consumer.dependencies) {
      inputCount += 1;
      const inputs = snapshot.edges.filter((edge) => edge.kind === "input" && edge.source === source && edge.slot === String(dependency.slot));
      assert.equal(inputs.length, 1, `Each input slot must have its own edge: ${source} / ${dependency.slot}`);
      const input = inputs[0];
      assert.equal(input.label, dependency.label);
      assert.equal(input.requested, dependency.requested);
      assert(snapshot.labels.some((text) => text.includes(dependency.label)), `Missing visible input label ${dependency.label}`);
      const [keyKind, keyValue] = keyParts(dependency.key);
      assert.equal(input.keyKind, keyKind);
      assert.equal(input.keyValue, keyValue);
      if (dependency.target === null) {
        assert(snapshot.nodes.some((node) => node.id === input.target && node.kind === "missing"), "Absent optional input must stay absent");
        continue;
      }
      const target = expectedId(dependency.target, entryId);
      const declaration = graph.nodes.find((node) => node.id === dependency.target);
      if (dependency.requested === declaration.name) {
        assert.equal(input.target, target, "Concrete input must retain its direct provider target");
        continue;
      }
      const matching = projections.filter((node) => node.id === input.target && node.target === target && node.requested === dependency.requested && node.keyKind === keyKind && node.keyValue === keyValue);
      assert.equal(matching.length, 1, `Expected one interface request for ${source} / ${dependency.slot} / ${keyKind}:${keyValue}`);
      const projection = matching[0];
      assert(!requests.has(projection.id), "Independent input slots must not collapse into one interface request");
      requests.add(projection.id);
      assert.equal(input.target, projection.id, "Trait input must terminate at its interface request");
      assert.notEqual(input.target, target, "Trait input must not bypass its interface request");
      const links = snapshot.edges.filter((edge) => edge.kind === "projection" && edge.source === projection.id && edge.target === target);
      assert.equal(links.length, 1, "One interface request has one selected implementation");
      assert.equal(links[0].slot, null, "Projection links are not additional input slots");
    }
  }
  assert.equal(projections.length, requests.size, "No duplicate or orphan interface requests");
  const ownIds = new Set(providers.map((node) => node.id));
  assert.equal(snapshot.edges.filter((edge) => edge.kind === "input" && ownIds.has(edge.source)).length, inputCount);
  return { snapshot, projections };
}

async function main() {
  const config = options(process.argv.slice(2));
  const modulePath = config.playwright || process.env.NESTRS_PLAYWRIGHT_MODULE;
  let playwright;
  try {
    playwright = require(modulePath ? path.resolve(modulePath) : "playwright");
  } catch (error) {
    throw new Error(`Playwright is unavailable; pass --playwright PATH or NESTRS_PLAYWRIGHT_MODULE. ${error.message}`);
  }
  const inputPath = path.resolve(config.html);
  const input = fs.readFileSync(inputPath, "utf8");
  const inputData = readGraph(input);
  const template = config.template ? fs.readFileSync(path.resolve(config.template), "utf8") : input;
  const graphs = inputData.version === 1 ? [inputData] : inputData.entries.filter((entry) => entry.status === "ok").map((entry) => entry.graph);
  const checkout = graphs.find((graph) => graph.nodes.some((node) => node.name.endsWith("::CheckoutService")));
  const checkoutEntry = inputData.version === 2 ? inputData.entries.find((entry) => entry.status === "ok" && entry.graph === checkout) : undefined;
  assert(checkout, "The input must contain the real checkout example graph");
  assert.equal(checkout.nodes.length, 10, "Expected the checkout example's ten provider declarations");
  const output = path.resolve(config.output);
  fs.mkdirSync(output, { recursive: true });
  const report = { passed: false, input: inputPath, template: config.template ? path.resolve(config.template) : inputPath, cases: [], errors: [], networkRequests: [] };
  const browser = await playwright.chromium.launch({ headless: true, executablePath: config.chromium || process.env.NESTRS_CHROMIUM_PATH, args: ["--no-sandbox"] });
  const context = await browser.newContext({ viewport: { width: 1600, height: 1100 }, offline: true });
  await context.addInitScript(() => { globalThis.__nestrsGraphInjected = false; });
  await context.route(/^https?:/, (route) => { report.networkRequests.push(route.request().url()); return route.abort(); });
  const page = await context.newPage();
  page.setDefaultTimeout(10000);
  page.on("pageerror", (error) => report.errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") report.errors.push(message.text()); });
  page.on("request", (request) => { if (/^https?:/.test(request.url())) report.networkRequests.push(request.url()); });

  async function load(name, graph) {
    const destination = path.join(output, `${name}.html`);
    fs.writeFileSync(destination, embedGraph(template, graph));
    await page.goto(pathToFileURL(destination).href, { waitUntil: "load" });
    await page.waitForFunction(() => document.querySelector("#visible-summary").textContent !== "正在读取依赖图…");
    assert.equal(await page.evaluate(() => globalThis.__nestrsGraphInjected), false, "Graph data executed as HTML or script");
    return destination;
  }

  async function checked(name, action) {
    await action();
    assert.deepEqual(report.errors, [], "The offline page must not emit JavaScript or console errors");
    assert.deepEqual(report.networkRequests, [], "Graph pages must work without network requests");
    assert.equal(await page.evaluate(() => globalThis.__nestrsGraphInjected), false);
    report.cases.push({ name, passed: true });
    process.stdout.write(`PASS ${name}\n`);
  }

  try {
    await checked("exported-input-page-at-desktop-widths", async () => {
      await load("exported-input", inputData);
      if (checkoutEntry) await page.locator("#entry-filter").selectOption(checkoutEntry.id);
      for (const width of [1600, 1280]) {
        await page.setViewportSize({ width, height: 1100 });
        await assertCounts(page, 10, 3, 1);
        await verifyVisibleGraph(page, checkout, checkoutEntry?.id);
        await assertGeometry(page);
        assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), "Exported input page has horizontal overflow");
        await page.screenshot({ path: path.join(output, `exported-input-${width}.png`), fullPage: true });
      }
      await page.setViewportSize({ width: 1600, height: 1100 });
    });

    await checked("checkout-trait-paths-and-key-isolation", async () => {
      await load("checkout-v1", checkout);
      await assertCounts(page, 10, 3, 1);
      const { projections } = await verifyVisibleGraph(page, checkout);
      assert.equal(projections.filter((node) => node.requested.endsWith("::OrderStore")).length, 1);
      const payments = projections.filter((node) => node.requested.endsWith("::PaymentGateway"));
      assert.deepEqual(payments.map((node) => node.keyValue).sort(), ["card", "wallet"]);
      assert.notEqual(payments[0].id, payments[1].id);
      await assertGeometry(page);
      await page.screenshot({ path: path.join(output, "checkout-overview.png"), fullPage: true });
      await selectNode(page, String(checkout.nodes.find((node) => node.name.endsWith("::CheckoutService")).id));
      await page.screenshot({ path: path.join(output, "checkout-selected.png"), fullPage: true });
      await selectNode(page, projections.find((node) => node.requested.endsWith("::OrderStore")).id);
      await page.screenshot({ path: path.join(output, "checkout-order-store-request.png"), fullPage: true });
      await selectNode(page, payments.find((node) => node.keyValue === "card").id);
      assert.match(await page.locator("#inspector-content").innerText(), /PaymentGateway/);
      assert.match(await page.locator("#inspector-content").innerText(), /PaymentClient/);
      assert.match(await page.locator("#inspector-content").innerText(), /card/);
      assert.equal(await page.locator("#inspector-id").innerText(), "接口请求");
      await page.locator("#related-button").click();
      assert.equal(await page.locator("#related-button").getAttribute("aria-pressed"), "true");
      const related = await state(page);
      assert(related.nodes.some((node) => node.kind === "projection" && node.requested.endsWith("::PaymentGateway")));
      assert(related.edges.some((edge) => edge.kind === "projection"));
      await page.screenshot({ path: path.join(output, "checkout-interface.png"), fullPage: true });
    });

    await checked("shared-target-independent-requests-repeated-slots-and-optional-input", async () => {
      const graph = syntheticGraph();
      await load("synthetic-v1", graph);
      await assertCounts(page, 5, 3, 1);
      const { snapshot, projections } = await verifyVisibleGraph(page, graph);
      const cardRequests = projections.filter((node) => node.keyValue === "card");
      assert.equal(cardRequests.length, 2);
      assert.notEqual(cardRequests[0].id, cardRequests[1].id);
      assert.equal(cardRequests[0].target, cardRequests[1].target);
      const card = cardRequests.find((node) => snapshot.edges.some((edge) => edge.kind === "input" && edge.source === "1" && edge.target === node.id));
      assert(card, "The second consumer needs its own interface request");
      assert.equal(snapshot.edges.filter((edge) => edge.kind === "input" && edge.target === card.id).length, 1);
      const duplicates = snapshot.edges.filter((edge) => edge.kind === "input" && edge.source === "0" && edge.target === "3");
      assert.deepEqual(duplicates.map((edge) => edge.slot).sort(), ["0", "1"]);
      await selectNode(page, card.id);
      const detail = await page.locator("#inspector-content").innerText();
      assert.match(detail, /CardImplementation/);
      assert(detail.includes("shared-card"), "Interface inspector must identify its own consumer slot");
      assert(!detail.includes("WalletImplementation"), "Different keyed implementation leaked into the card route");
      await page.locator("#service-search").fill("Missing");
      const missingResult = page.locator("#search-results .search-result").filter({ hasText: "Missing" });
      assert(await missingResult.count() >= 1, "Missing optional requests must be searchable");
      await missingResult.last().click();
      assert.match(await page.locator("#inspector-content").innerText(), /Missing/);
      assert.match(await page.locator("#inspector-content").innerText(), /缺席|未注册|不存在/);
      await page.locator("#service-search").fill("fixture::Port");
      const interfaceResult = page.locator("#search-results .search-result").filter({ hasText: "接口请求" });
      assert.equal(await interfaceResult.count(), 3, "Every interface request must appear in search");
      await interfaceResult.first().click();
      assert.equal(await page.locator("#inspector-id").innerText(), "接口请求");
    });

    await checked("service-initialization-is-distinct-from-lazy-fields", async () => {
      const graph = syntheticGraph();
      graph.nodes[0].initialization = "lazy";
      graph.nodes[1].initialization = "eager";
      graph.nodes[2].initialization = "lazy";
      graph.nodes[3].initialization = "lazy";
      graph.nodes[4].initialization = "eager";
      graph.nodes[0].dependencies[4].lazy = true;
      await load("initialization-policies", graph);
      await assertCounts(page, 5, 3, 1);
      const { snapshot } = await verifyVisibleGraph(page, graph);
      assert.equal(snapshot.nodes.find((node) => node.id === "2").initialization, "lazy");
      assert.equal(snapshot.nodes.find((node) => node.id === "4").initialization, "eager");
      assert.equal(snapshot.edges.find((edge) => edge.source === "0" && edge.slot === "3").lazy, "false", "Lazy service policy must not delay an ordinary injected field");
      assert.equal(snapshot.edges.find((edge) => edge.source === "0" && edge.slot === "4").lazy, "true", "Field laziness must remain independent of the eager target service");
      await selectNode(page, "2");
      assert.match(await page.locator("#inspector-content").innerText(), /普通依赖需要它时仍会构造/);
      await selectNode(page, "4");
      assert.match(await page.locator("#inspector-content").innerText(), /覆盖 root 的 Lazy/);
      await selectNode(page, "0");
      assert.match(await page.locator("#inspector-content").innerText(), /不作为scope 创建时的初始化入口/);
      await selectNode(page, "1");
      assert.match(await page.locator("#inspector-content").innerText(), /create_scope\(options\)\.await 返回前主动初始化，覆盖本次 scope 的 Lazy/);
      await selectNode(page, "3");
      assert.match(await page.locator("#inspector-content").innerText(), /不增加独立预热实例/);
      const textOverflow = await page.locator(".node-initialization").evaluateAll((labels) => labels.filter((label) => {
        const text = label.getBBox();
        const frame = label.parentElement.querySelector(".node-frame").getBBox();
        return text.x < frame.x || text.y < frame.y || text.x + text.width > frame.x + frame.width || text.y + text.height > frame.y + frame.height;
      }).map((label) => label.textContent));
      assert.deepEqual(textOverflow, [], "Service initialization labels must fit inside their provider nodes");
      await page.screenshot({ path: path.join(output, "initialization-policies.png"), fullPage: true });
      await page.locator("#service-search").fill("提前初始化");
      assert.equal(await page.locator("#search-results .search-result[data-node-kind=provider]").count(), 2);
    });

    await checked("old-graphs-inherit-policy-and-project-declarations-retain-policy", async () => {
      const graph = syntheticGraph();
      await load("initialization-backward-compatible", graph);
      assert((await state(page)).nodes.filter((node) => node.kind === "provider").every((node) => node.initialization === "inherit"));
      await selectNode(page, "2");
      assert.match(await page.locator("#inspector-content").innerText(), /继承配置/);
      await selectNode(page, "0");
      assert.match(await page.locator("#inspector-content").innerText(), /由本次 scope 的初始化配置决定/);
      assert.match(await page.locator("#inspector-content").innerText(), /root 的初始化模式不改变 scope 默认值/);
      const project = projectGraph(graph);
      project.entries[1].graph.nodes[2].initialization = "lazy";
      project.entries[1].graph.nodes[4].initialization = "inherit";
      await load("initialization-project-identity", project);
      assert.match(await page.locator("#project-summary").innerText(), /4 个声明在多个入口中出现/);
      await selectNode(page, expectedId(2, "entry-a"));
      assert.equal(await page.locator("#inspector-content .badge.shared").count(), 0, "Different initialization policies must not appear as the same declaration");
      await selectNode(page, expectedId(4, "entry-a"));
      assert.equal(await page.locator("#inspector-content .badge.shared").count(), 1, "Missing and explicit inherit policy have the same meaning");
    });

    await checked("named-and-indexed-keys-do-not-collapse", async () => {
      const graph = syntheticGraph();
      const replace = (key) => key === null ? null : key.value === "card" ? { kind: "indexed", value: "0" } : NAMED("0");
      for (const node of graph.nodes) {
        node.key = replace(node.key);
        for (const dependency of node.dependencies) dependency.key = replace(dependency.key);
      }
      await load("synthetic-key-kinds", graph);
      const { projections } = await verifyVisibleGraph(page, graph);
      assert.deepEqual(projections.map((node) => node.keyKind).sort(), ["indexed", "indexed", "named"]);
      assert(projections.every((node) => node.keyValue === "0"));
      assert.notEqual(projections[0].id, projections[1].id);
    });

    await checked("indexed-keys-retain-decimal-string-precision", async () => {
      const graph = syntheticGraph();
      const first = "9007199254740992";
      const adjacent = "9007199254740993";
      assert.equal(Number(first), Number(adjacent), "Fixture must exercise values that lose precision as JavaScript numbers");
      const replace = (key) => key === null ? null : { kind: "indexed", value: key.value === "card" ? first : adjacent };
      for (const node of graph.nodes) {
        node.key = replace(node.key);
        for (const dependency of node.dependencies) dependency.key = replace(dependency.key);
      }
      // A named key with identical text is a third independent registration.
      const named = { ...structuredClone(graph.nodes[2]), id: 5, key: NAMED(first) };
      graph.nodes.push(named);
      graph.nodes[0].dependencies.push({
        slot: 5, label: "named-large", target: named.id, requested: "dyn fixture::Port",
        requestedLabel: "dyn Port", key: NAMED(first), optional: false,
      });
      await load("synthetic-large-indexed-keys", graph);
      await assertCounts(page, 6, 4, 1);
      const { projections } = await verifyVisibleGraph(page, graph);
      const identities = new Set(projections.map((node) => JSON.stringify([node.keyKind, node.keyValue, node.target])));
      assert.equal(identities.size, 3, "Adjacent indexed values and the named equivalent must remain distinct");
      assert.equal(projections.filter((node) => node.keyKind === "indexed" && node.keyValue === first).length, 2);
      assert.equal(projections.filter((node) => node.keyKind === "indexed" && node.keyValue === adjacent).length, 1);
      assert.equal(projections.filter((node) => node.keyKind === "named" && node.keyValue === first).length, 1);
      for (const key of [first, adjacent]) {
        await page.locator("#service-search").fill(key);
        const results = page.locator("#search-results .search-result[data-node-kind=projection]");
        assert.equal(await results.count(), key === first ? 3 : 1, "Key search must preserve exact decimal text");
      }
      await page.locator("#search-results .search-result[data-node-kind=projection]").click();
      assert((await page.locator("#inspector-content").innerText()).includes(adjacent), "Inspector rounded an indexed key");
    });

    await checked("project-entry-isolation-filters-search-and-safe-text", async () => {
      const graph = syntheticGraph();
      const project = projectGraph(graph);
      await load("synthetic-project", project);
      await assertCounts(page, 10, 6, 2);
      await verifyVisibleGraph(page, graph, "entry-a");
      await verifyVisibleGraph(page, graph, "entry-b");
      const before = await state(page);
      assert.equal(new Set(before.nodes.map((node) => node.id)).size, before.nodes.length, "Local provider IDs must be namespaced per entry");
      for (const edge of before.edges) {
        const source = before.nodes.find((node) => node.id === edge.source);
        const target = before.nodes.find((node) => node.id === edge.target);
        assert.equal(source.entryId, target.entryId, "Graph edge crossed independent entry boundaries");
      }
      await page.locator("#entry-filter").selectOption("entry-a");
      await assertCounts(page, 5, 3, 1);
      await verifyVisibleGraph(page, graph, "entry-a");
      assert((await state(page)).nodes.every((node) => node.entryId === "entry-a"));
      await page.locator("#service-search").fill("fixture::Port");
      assert.equal(await page.locator("#search-results .search-result").filter({ hasText: "接口请求" }).count(), 3);
      await page.locator("#entry-filter").selectOption("entry-error");
      assert.equal((await state(page)).nodes.length, 0);
      assert(await page.locator("#inspector-content").innerText().then((text) => text.includes(ATTACK)), "Diagnostics must be literal text");
      assert.equal(await page.locator("img,script[src]").count(), 0, "Untrusted graph text created an executable element");
      await page.locator("#entry-filter").selectOption("entry-skipped");
      assert.equal((await state(page)).nodes.length, 0);
      assert.match(await page.locator("#inspector-content").innerText(), /extra|feature/);
      await page.locator("#entry-filter").selectOption("");
      await assertCounts(page, 10, 6, 2);
      await page.screenshot({ path: path.join(output, "project-overview.png"), fullPage: true });
    });

    await checked("all-error-and-skipped-report", async () => {
      await load("no-valid-entries", projectGraph(syntheticGraph(), false));
      await assertCounts(page, 0, 0, 0);
      assert.equal((await state(page)).nodes.length, 0);
      assert.match(await page.locator("#project-summary").innerText(), /0 个结构有效/);
      assert.equal(await page.locator(".entry-card[data-status=error]").count(), 1);
      assert.equal(await page.locator(".entry-card[data-status=skipped]").count(), 1);
      await page.locator("#entry-filter").selectOption("entry-error");
      assert((await page.locator("#inspector-content").innerText()).includes(ATTACK));
    });

    await checked("large-project-hidden-input-search-related-view-and-pagination", async () => {
      const graph = largeGraph();
      await load("large-project", projectGraph(graph));
      await page.locator("#entry-filter").selectOption("entry-a");
      await assertCounts(page, 2, 218, 80);
      const initial = await state(page);
      assert(initial.nodes.length <= 180, "A local view exceeded the existing node budget");
      assert(initial.nodes.every((node) => node.entryId === "entry-a"));
      assert(!initial.nodes.some((node) => node.requested === "dyn fixture::LateInterface"), "Test requires a request outside the initial local view");
      assert(await page.locator("#graph-notice").isVisible());
      assert.match(await page.locator("#graph-notice").innerText(), /180|局部/);
      await page.locator("#service-search").fill("LateInterface");
      await page.locator("#search-results .search-result[data-node-kind=projection]").click();
      const selected = await state(page);
      const request = selected.nodes.find((node) => node.requested === "dyn fixture::LateInterface");
      assert(request, "Search did not reveal an omitted interface request");
      const owner = expectedId(0, "entry-a");
      const target = expectedId(1, "entry-a");
      assert(selected.nodes.some((node) => node.id === owner), "Interface context omitted its consumer");
      assert(selected.nodes.some((node) => node.id === target), "Interface context omitted its selected provider");
      assert(selected.edges.some((edge) => edge.kind === "input" && edge.source === owner && edge.target === request.id && edge.slot === "219"));
      assert(selected.edges.some((edge) => edge.kind === "projection" && edge.source === request.id && edge.target === target));
      await page.locator("#related-button").click();
      assert((await state(page)).nodes.some((node) => node.id === request.id));
      await page.locator("#service-search").fill("LateOptional");
      await page.locator("#search-results .search-result[data-node-kind=missing]").click();
      assert.equal(await page.locator("#inspector-id").innerText(), "可选输入缺席");
      assert.match(await page.locator("#inspector-content").innerText(), /299/);
      assert((await state(page)).nodes.some((node) => node.id === owner), "Optional context omitted its consumer");
      await selectNode(page, owner);
      assert.equal(await page.locator("#inspector-content .dependency-list .dependency-card").count(), 40);
      for (let count = 0; count < 7; count += 1) {
        const more = page.getByRole("button", { name: /^显示更多输入/ });
        if (await more.count() === 0) break;
        await more.click();
      }
      const slots = await page.locator("#inspector-content .dependency-list .dependency-slot").allTextContents();
      assert.deepEqual(slots, Array.from({ length: 300 }, (_, slot) => `slot ${slot}`), "Dependency pagination lost or collapsed input slots");
      const firstTargets = await page.locator("#inspector-content .dependency-list .dependency-card").evaluateAll((cards) => cards.slice(0, 2).map((card) => card.getAttribute("data-dependency-target")));
      assert.deepEqual(firstTargets, [target, target], "Repeated Transient inputs must retain independent cards");
      await page.locator("#entry-filter").selectOption("entry-b");
      assert((await state(page)).nodes.every((node) => node.entryId === "entry-b"), "Local graph retained nodes from the old entry");
      await page.screenshot({ path: path.join(output, "large-project-local-view.png"), fullPage: true });
    });

    await checked("mobile-checkout-interface-navigation", async () => {
      await page.setViewportSize({ width: 390, height: 844 });
      await load("checkout-mobile", checkout);
      await assertCounts(page, 10, 3, 1);
      await page.locator("#service-search").fill("OrderStore");
      const result = page.locator("#search-results .search-result").filter({ hasText: "接口请求" });
      assert.equal(await result.count(), 1);
      await result.click();
      assert.match(await page.locator("#inspector-content").innerText(), /OrderStore/);
      assert.match(await page.locator("#inspector-content").innerText(), /Repository/);
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), "Mobile page has horizontal overflow");
      await page.screenshot({ path: path.join(output, "checkout-mobile.png"), fullPage: true });
    });
    report.passed = true;
  } catch (error) {
    report.failure = error.stack || String(error);
    await page.screenshot({ path: path.join(output, "failure.png"), fullPage: true }).catch(() => {});
    throw error;
  } finally {
    fs.writeFileSync(path.join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
    await browser.close();
  }
  process.stdout.write(`Verified ${report.cases.length} browser scenarios; report: ${path.join(output, "report.json")}\n`);
}

main().catch((error) => {
  process.stderr.write(`error: ${error.stack || error}\n`);
  process.exitCode = 1;
});
