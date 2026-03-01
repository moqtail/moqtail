import prompts from 'prompts'
import { execSync } from 'child_process'

async function main() {
  try {
    const modifiedFiles = execSync('git status --porcelain', { encoding: 'utf-8' })
      .split('\n')
      .filter((line) => line.trim().length > 0)

    if (modifiedFiles.length > 0) {
      if (!process.stdin.isTTY || !process.stdout.isTTY) {
        console.log('No interactive terminal available, skipping changeset prompt')
        return
      }

      const response = await prompts(
        {
          type: 'confirm',
          name: 'addChangeset',
          message: 'There are modified files. Do you want to add a changeset now?',
          initial: true,
        },
        {
          onCancel: () => {
            process.exit(0)
          },
        },
      )

      if (response.addChangeset) {
        execSync('npm run changeset', { stdio: 'inherit' })
        execSync('git add .changeset', { stdio: 'inherit' })
      }
    }
  } catch (error) {
    console.error('Error in precommit script:', error.message)
    process.exit(1)
  }
}

main().catch((error) => {
  console.error('Unhandled error:', error)
  process.exit(1)
})
