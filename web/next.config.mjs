/** @type {import('next').NextConfig} */
const nextConfig = {
  // Allow cross-origin requests to Souveraine server in dev
  async rewrites() {
    return [
      {
        source: '/api/souveraine/:path*',
        destination: 'http://localhost:8484/v1/:path*',
      },
    ];
  },
};

export default nextConfig;
