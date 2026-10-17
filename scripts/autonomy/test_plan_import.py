"""Fixture tests for plan_import.py (doc 066 Q4). Never touches the real queue: every path is a temp override."""
import hashlib, os, subprocess, sys, tempfile, unittest, fcntl

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "plan_import.py")
JOURNAL = "pub const DEFAULT_BATCH: usize = 16;"
BUDGET = 'assert!(x, "a run costs {per_run}/100 commits");'


def block(**kw):
    d = dict(id="065-Q1", rev="", title="the barrier is a deadline", cls="mechanical", spec_slug="the-barrier-is-a-deadline",
             files="crates/pii-graph/src/journal.rs", criteria="cargo test -p pii-graph --test write_amp",
             evidence=['crates/pii-graph/src/journal.rs:2 "DEFAULT_BATCH"'], body="Flush on a deadline.")
    d.update(kw)
    ev = "".join(f"evidence: {e}\n" for e in d["evidence"])
    return (f"```queue\nid: {d['id']}\nrev: {d['rev']}\ntitle: {d['title']}\nclass: {d['cls']}\nspec_slug: {d['spec_slug']}\n"
            f"files: {d['files']}\ncriteria: {d['criteria']}\n{ev}body: {d['body']}\n```\n")


class Fx:
    def __init__(self):
        self.t = tempfile.mkdtemp(prefix="plan-import-test-", dir=os.environ.get("TMPDIR") or "/tmp")
        self.root = self.t
        w = lambda rel, txt: (os.makedirs(os.path.dirname(os.path.join(self.t, rel)), exist_ok=True),
                              open(os.path.join(self.t, rel), "w").write(txt))
        w("crates/pii-graph/src/journal.rs", f"// j\n{JOURNAL}\n")
        w("crates/pii-graph/tests/write_amp.rs", f"// w\n{BUDGET}\n")
        w("specs/100-old.md", "# old\n")
        w("plan.tsv", "# hdr\nA1\t100-old.md\t-\told\tx\tcrates/a\tq\nA2\t100-old.md\t-\told\tx\tcrates/a\tq\n")
        w("prov.tsv", "")
        w("questions.md", "")
        w("review.md", "")
        self.env = dict(os.environ, PII_ROOT=self.t, PII_PLAN=f"{self.t}/plan.tsv", PII_PROVENANCE=f"{self.t}/prov.tsv",
                        PII_QUESTIONS=f"{self.t}/questions.md", PII_SPECS=f"{self.t}/specs", PII_LOCK=f"{self.t}/lock",
                        PII_ACTS=f"{self.t}/acts.log")

    def doc(self, *blocks):
        open(f"{self.t}/review.md", "w").write("prose\n" + "\n".join(blocks))

    def run(self, *a):
        r = subprocess.run([sys.executable, TOOL, *a], env=self.env, capture_output=True, text=True, cwd=self.t)
        return r.returncode, r.stderr + r.stdout

    def imp(self, *a):
        return self.run("import", f"{self.t}/review.md", *a)

    def snap(self):
        h = hashlib.sha256()
        for rel in ("plan.tsv", "prov.tsv") + tuple(sorted("specs/" + f for f in os.listdir(f"{self.t}/specs"))):
            h.update(open(f"{self.t}/{rel}", "rb").read())
        return h.hexdigest()

    def lines(self, f):
        return len([l for l in open(f"{self.t}/{f}") if l.strip() and not l.startswith("#")])


class Refusals(unittest.TestCase):
    def setUp(self):
        self.f = Fx()

    def refuse(self, b, code, text):
        self.f.doc(b)
        before = self.f.snap()
        rc, out = self.f.imp()
        self.assertEqual(rc, code, out)
        self.assertIn(text, out)
        self.assertEqual(before, self.f.snap(), "a refusal must change nothing")

    def test_r1_duplicate_id(self):
        self.f.doc(block())
        self.assertEqual(self.f.imp()[0], 0)
        self.f.doc(block(body="changed", spec_slug="other-slug"))
        rc, out = self.f.imp()
        self.assertEqual(rc, 2)
        self.assertIn("duplicate: 065-Q1 already queued as A3", out)

    def test_r2_duplicate_slug(self):
        self.f.doc(block())
        self.f.imp()
        self.refuse(block(id="065-Q2"), 2, "duplicate spec slug: the-barrier-is-a-deadline")

    def test_r3_no_evidence(self):
        self.refuse(block(evidence=[]), 2, "no evidence: 065-Q1")

    def test_r4_missing_file(self):
        self.refuse(block(evidence=['crates/x/y.rs:1 "a"']), 2, "evidence missing: crates/x/y.rs")

    def test_r5_moved(self):
        p = f"{self.f.t}/crates/pii-graph/src/journal.rs"
        open(p, "w").write(f"// j\n// new\n{JOURNAL}\n")
        self.refuse(block(), 2, "moved: journal.rs:2 -> :3")

    def test_r6_stale(self):
        open(f"{self.f.t}/crates/pii-graph/src/journal.rs", "w").write("// j\nnothing\n")
        self.refuse(block(), 2, 'stale: journal.rs:2 no longer reads "DEFAULT_BATCH"')

    def test_r7_deploy_escalates_whatever_the_label(self):
        for cls in ("mechanical", "decision"):
            self.refuse(block(files="deploy/install-pii.sh", cls=cls), 3, "escalated: 065-Q1")
        self.assertEqual(self.f.lines("questions.md"), 1, "asked once, not per run")

    def test_r8_budget_file_escalates(self):
        self.refuse(block(files="crates/pii-graph/tests/write_amp.rs"), 3, "write_amp.rs")

    def test_r9_prose_criterion(self):
        self.refuse(block(criteria="the run is visible"), 2, "criterion is not a command: 065-Q1")

    def test_r12_lock_held(self):
        self.f.doc(block())
        fh = open(f"{self.f.t}/lock", "w")
        fcntl.flock(fh, fcntl.LOCK_EX)
        rc, _ = self.f.imp()
        self.assertEqual(rc, 75)
        self.assertEqual(self.f.lines("plan.tsv"), 2)

    def test_r10_r11_check_flags_hand_rows(self):
        w = lambda rel, t: open(f"{self.f.t}/{rel}", "w").write(t)
        w("specs/161-x.md", "# s\n\n## Exit Criteria\n\n- [ ] `cargo a`\n- [ ] `cargo b`\n")
        w("plan.tsv", "A28\t161-x.md\t-\tt\tcargo a\tcrates/pii-learn\tq\n")
        rc, out = self.f.run("check")
        self.assertEqual(rc, 2)
        self.assertIn("A28 verify differs from specs/161 exit criteria", out)
        self.assertIn("A28 commit paths do not cover the spec's files", out)

    def test_r13_hand_edited_spec(self):
        self.f.doc(block())
        self.f.imp()
        self.assertNotIn("A3", self.f.run("check")[1])
        with open(f"{self.f.t}/specs/101-the-barrier-is-a-deadline.md", "a") as fh:
            fh.write("hand edit\n")
        rc, out = self.f.run("check")
        self.assertEqual(rc, 2)
        self.assertIn("plan-error: A3 specs/101 differs from its render of 065-Q1", out)


class Accepts(unittest.TestCase):
    def setUp(self):
        self.f = Fx()

    def test_a1_a2_one_row_then_idempotent(self):
        self.f.doc(block())
        rc, out = self.f.imp()
        self.assertEqual(rc, 0, out)
        self.assertEqual((self.f.lines("plan.tsv"), self.f.lines("prov.tsv")), (3, 1))
        row = [l for l in open(f"{self.f.t}/plan.tsv")][-1].split("\t")
        self.assertEqual(row[0], "A3")
        self.assertEqual(row[4], "cargo test -p pii-graph --test write_amp")
        self.assertEqual(row[5], "crates/pii-graph/src/journal.rs")
        self.assertTrue(os.path.isfile(f"{self.f.t}/specs/101-the-barrier-is-a-deadline.md"))
        before = self.f.snap()
        rc, out = self.f.imp()
        self.assertEqual(rc, 0)
        self.assertIn("already queued", out)
        self.assertEqual(before, self.f.snap())

    def test_a3_criteria_may_run_a_budget_test(self):
        self.f.doc(block(criteria="cargo test -p pii-graph --test write_amp"))
        self.assertEqual(self.f.imp()[0], 0)

    def test_a4_gate_steps_add_queues_remove_escalates(self):
        open(f"{self.f.t}/gate-steps.txt", "w").write("x\n")
        self.f.doc(block(files="gate-steps.txt (add)", evidence=['gate-steps.txt:1 "x"']))
        self.assertEqual(self.f.imp()[0], 0)
        self.f.doc(block(id="065-Q2", spec_slug="s2", files="gate-steps.txt (remove)", evidence=['gate-steps.txt:1 "x"']))
        self.assertEqual(self.f.imp()[0], 3)

    def test_a5_approved_queues_and_logs(self):
        self.f.doc(block(files="deploy/install-pii.sh"))
        self.assertEqual(self.f.imp()[0], 3)
        rc, out = self.f.imp("--approved", "065-Q1")
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.f.lines("plan.tsv"), 3)
        self.assertEqual(self.f.lines("acts.log"), 1)

    def test_label_independence(self):
        for cls in ("mechanical", "decision"):
            f = Fx()
            f.doc(block(files="crates/pii-graph/tests/write_amp.rs", cls=cls))
            self.assertEqual(f.imp()[0], 3)


if __name__ == "__main__":
    unittest.main()
