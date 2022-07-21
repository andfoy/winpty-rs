To release a new version of winpty-rs:

1. git fetch upstream && git checkout upstream/main
2. Close milestone on GitHub
3. git clean -xfdi
4. Update CHANGELOG.md with loghub
5. git add -A && git commit -m "Update Changelog"
6. Update release version in ``Cargo.toml`` (set release version, remove 'dev')
7. Update version in README
8. git add -A && git commit -m "Release vX.X.X"
9. git tag -a vX.X.X -m "Release vX.X.X"
10. Update development version in ``Cargo.toml`` (add '-dev' and increment minor version)
11. git add -A && git commit -m "Set development version to vY.Y.Y"
12. git push upstream main
13. git push upstream --tags
14. Create release in GitHub
15. Wait for GitHub actions to publish on crates.io
