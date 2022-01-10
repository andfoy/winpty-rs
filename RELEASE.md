To release a new version of winpty-rs:

1. git fetch upstream && git checkout upstream/master
2. Close milestone on GitHub
3. git clean -xfdi
4. Update CHANGELOG.md with loghub
5. git add -A && git commit -m "Update Changelog"
6. Update release version in ``Cargo.toml`` (set release version, remove 'dev')
7. git add -A && git commit -m "Release vX.X.X"
10. git tag -a vX.X.X -m "Release vX.X.X"
11. Update development version in ``Cargo.toml`` (add '-dev' and increment minor version)
12. git add -A && git commit -m "Back to work"
13. git push upstream master
14. git push upstream --tags
15. Create release in GitHub
16. Wait for GitHub actions to publish on crates.io
