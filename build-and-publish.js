const { execSync } = require("child_process");
const readline = require("readline");
const fs = require("fs");
const path = require("path");

const cliCommitMessage = process.argv[2];

function runBuildAndPublish(commitMessage) {
    try {
        // ── 1. Tests ─────────────────────────────────────────────────────
        console.log("🧪 Running Rust tests...");
        execSync("cargo test --all-features", { stdio: "inherit" });

        // ── 2. Build WASM ─────────────────────────────────────────────────
        console.log("📦 Building WASM...");
        execSync("wasm-pack build --target web --scope killriam --release", { stdio: "inherit" });

        // ── 3. Patch pkg/package.json with GitHub registry ───────────────
        const pkgPath = path.join(__dirname, "pkg", "package.json");
        const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
        pkg.publishConfig = { registry: "https://npm.pkg.github.com/" };
        fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2));
        console.log("✏️  Patched pkg/package.json with GitHub registry");

        // ── 4. Bump version in Cargo.toml ─────────────────────────────────
        console.log("🔖 Bumping version...");
        const cargoToml = fs.readFileSync("Cargo.toml", "utf8");
        const match = cargoToml.match(/^version = "(\d+)\.(\d+)\.(\d+)"/m);
        if (!match) throw new Error("Could not find version in Cargo.toml");
        const [, maj, min, pat] = match;
        const newVersion = `${maj}.${min}.${parseInt(pat) + 1}`;
        const oldVersion = `${maj}.${min}.${pat}`;
        fs.writeFileSync("Cargo.toml", cargoToml.replace(
            `version = "${oldVersion}"`,
            `version = "${newVersion}"`
        ));
        pkg.version = newVersion;
        fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2));
        console.log(`   ${oldVersion} → ${newVersion}`);

        // ── 5. Git commit + tag ───────────────────────────────────────────
        console.log("📤 Committing...");
        execSync("git add Cargo.toml Cargo.lock", { stdio: "inherit" });
        try {
            execSync(`git commit -m "${commitMessage} (v${newVersion})"`, { stdio: "inherit" });
        } catch {
            console.log("No changes to commit, continuing...");
        }

        console.log("🚀 Pushing to GitHub...");
        execSync("git push origin master", { stdio: "inherit" });

        execSync(`git tag v${newVersion}`, { stdio: "inherit" });
        execSync("git push --tags", { stdio: "inherit" });

        // ── 6. Publish ────────────────────────────────────────────────────
        console.log(`🚀 Publishing @killriam/mamo-sim@${newVersion}...`);
        execSync("dotenv -- npm publish --access public", {
            stdio: "inherit",
            cwd: path.join(__dirname, "pkg"),
        });

        console.log(`✅ Published @killriam/mamo-sim@${newVersion} to GitHub Package Registry!`);
        console.log(`\n   Update frontend:\n   npm install @killriam/mamo-sim@${newVersion}`);
    } catch (error) {
        console.error("❌ Error:", error.message);
        process.exit(1);
    }
}

if (cliCommitMessage) {
    console.log(`📝 Commit message: "${cliCommitMessage}"`);
    runBuildAndPublish(cliCommitMessage);
} else {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question("Enter commit message (or press Enter for default): ", (input) => {
        runBuildAndPublish(input.trim() || "chore: release mamo-sim");
        rl.close();
    });
}
