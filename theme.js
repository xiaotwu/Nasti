try {
  const preference = localStorage.getItem("nasti-site-theme");
  if (preference === "dark" || preference === "light") {
    document.documentElement.dataset.theme = preference;
  } else if (matchMedia("(prefers-color-scheme: dark)").matches) {
    document.documentElement.dataset.theme = "dark";
  }
} catch { /* The page keeps its rendered theme when storage is unavailable. */ }
