/**
 * @filename: lint-staged.config.js
 * @type {import('lint-staged').Configuration}
 */
export default {
  '(apps|libs)/**/*.{rs,toml}': (files) => {
    const projects = files.reduce((acc, file) => {
      const match = file.match(/^((apps|libs)\/[^/]+)/)
      if (match) {
        const project = match[1]
        if (!acc.includes(project)) acc.push(project)
      }
      return acc
    }, [])
    return projects.flatMap((project) => {
      const manifestPathArg = `--manifest-path=${project}/Cargo.toml`
      const all = `--all-targets --all-features`
      return [
        // Run cargo clippy once for every changed project
        `cargo clippy ${manifestPathArg} ${all} --fix --allow-dirty -- -D warnings`,
        // Run cargo fmt once for every changed project
        `cargo fmt ${manifestPathArg}`,
        // Run cargo build once for every changed project
        `cargo build ${manifestPathArg} ${all}`,
        // Run cargo check once for every changed project
        `cargo check ${manifestPathArg} ${all}`,
        // Run cargo test once for every changed project
        `cargo test ${manifestPathArg} ${all}`,
      ]
    })
  },

  // Run build/test for all changed projects (at once)
  '(apps|libs)/**/*.ts*': () => [
    'npm run build', // The order of builds matter
    'npm run test',
  ],

  // Run prettier for rest of the files
  '*': ['prettier --ignore-unknown --write'],
}
