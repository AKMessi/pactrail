
rgtest!(
    pactrail_hidden_early_termination_byte_stats,
    |dir: Dir, mut cmd: TestCommand| {
        dir.create("haystack", "foo1\nfoo2\nfoo3\nfoo4\nfoo5\n");
        let output = cmd.args(&["--stats", "-m2", "foo", "."]).stdout();
        assert!(
            output.contains("10 bytes searched\n"),
            "unexpected statistics output:\n{output}"
        );
    }
);
