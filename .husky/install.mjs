// Skip Husky install in production and CI
if (process.env.NODE_ENV === 'production' || process.env.CI === 'true') {
  process.exit(0);
}

// Install Husky hooks
try {
  console.log('Installing Husky...');
  const husky = (await import('husky')).default;
  const output = husky();

  if (output && output.length > 0) {
    console.log(output);
  }
} catch (error) {
  if (error.code !== 'ERR_MODULE_NOT_FOUND') {
    throw error;
  }
}
