import "./globals.css";

export const metadata = {
  title: "Ani Studio — Personal AI Interface",
  description: "A premium web interface for the Souveraine consciousness substrate. Interact with your personal AI agent in a beautiful, responsive environment.",
  icons: {
    icon: '/favicon.ico',
  },
};

export default function RootLayout({ children }) {
  return (
    <html lang="en">
      <head>
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <meta name="theme-color" content="#07070d" />
      </head>
      <body>
        {children}
      </body>
    </html>
  );
}
