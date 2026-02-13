import { glob } from 'glob'
import { exec } from 'child_process'
import { readFile, writeFile } from 'fs/promises'
import { dirname, join, basename } from 'path'

async function updateCargoTomlVersions() {
  // Find all Cargo.toml files not in the root directory
  const cargoTomlPaths = await glob('**/Cargo.toml', {
    ignore: ['Cargo.toml', 'node_modules/**'],
    cwd: process.cwd(),
    absolute: true,
  })

  for (const cargoTomlPath of cargoTomlPaths) {
    const dir = dirname(cargoTomlPath)
    const pkgJsonPath = join(dir, 'package.json')

    try {
      // Read package.json
      const pkgJson = JSON.parse(await readFile(pkgJsonPath, 'utf8'))
      const version = pkgJson.version
      if (!version) continue

      // Read Cargo.toml
      let cargoToml = await readFile(cargoTomlPath, 'utf8')

      // Replace version in Cargo.toml (assumes [package] section and version = "..."
      cargoToml = cargoToml.replace(/(^|\n)version\s*=\s*"(.*?)"/, `$1version = "${version}"`)

      // Write back Cargo.toml
      await writeFile(cargoTomlPath, cargoToml, 'utf8')
      console.log(`Updated ${basename(cargoTomlPath)} in ${dir} to version ${version}`)
    } catch (err) {
      // Ignore if package.json doesn't exist or parse fails
      continue
    }
  }

  // Run cargo update
  exec('cargo update -w', (error, stdout, stderr) => {
    if (error) {
      console.error(`Error running cargo update: ${error.message}`)
      process.exit(1)
    }
  })

  // Update package-lock.json
  exec('npm install --package-lock-only', (error, stdout, stderr) => {
    if (error) {
      console.error(`Error running npm install --package-lock-only: ${error.message}`)
      process.exit(1)
    }
  })
}

await updateCargoTomlVersions()
