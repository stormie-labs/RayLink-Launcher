// RayLink Setup: puts RayLink Launcher into a folder of the player's choice, with shortcuts and an entry in Windows'
// "Apps" list for removal. The launcher itself is inside (a resource); the build, the scripts and the stand come later
// from the launcher. No admin rights: the default folder is %LOCALAPPDATA%\Programs\RayLink, the entry is per user.
//   RayLinkSetup.exe                 the window
//   RayLinkSetup.exe /S /D=<folder>  silent install (tests)
//   uninstall.exe /uninstall         (the copy of this program in the install folder) removes the launcher
using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Reflection;
using System.Windows.Forms;
using Microsoft.Win32;

[assembly: AssemblyTitle("RayLink Setup")]
[assembly: AssemblyProduct("RayLink")]
[assembly: AssemblyVersion("0.1.0.0")]
[assembly: AssemblyFileVersion("0.1.0.0")]
[assembly: System.Runtime.Versioning.TargetFramework(".NETFramework,Version=v4.8", FrameworkDisplayName = ".NET Framework 4.8")]

namespace RayLink
{
	static class Setup
	{
		const string Version = "0.1.0";
		const string Key = @"Software\Microsoft\Windows\CurrentVersion\Uninstall\RayLink";
		const string Exe = "RayLinkLauncher.exe";

		static string DefaultDir
		{
			get { return Path.Combine(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs"), "RayLink"); }
		}

		static string DesktopLink { get { return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory), "RayLink.lnk"); } }
		static string MenuLink { get { return Path.Combine(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "RayLink"), "RayLink.lnk"); } }

		[STAThread]
		static int Main(string[] args)
		{
			try
			{
				if (args.Length > 0 && args[0].ToLowerInvariant() == "/uninstall") return Uninstall(args.Length > 1 && args[1] == "/S");
				if (args.Length > 0 && args[0] == "/S")
				{
					string dir = DefaultDir;
					foreach (string a in args) if (a.StartsWith("/D=")) dir = a.Substring(3);
					Install(dir, true, true);
					return 0;
				}
				Application.EnableVisualStyles();
				Application.Run(new SetupForm());
				return 0;
			}
			catch (Exception e)
			{
				if (args.Length > 0 && (args[0] == "/S" || (args.Length > 1 && args[1] == "/S"))) Console.Error.WriteLine(e.Message);
				else MessageBox.Show(e.Message, "RayLink Setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
				return 1;
			}
		}

		public static string Install(string dir, bool desktop, bool menu)
		{
			Directory.CreateDirectory(dir);
			string exe = Path.Combine(dir, Exe);
			using (Stream s = Assembly.GetExecutingAssembly().GetManifestResourceStream("launcher.exe"))
			{
				if (s == null) throw new IOException("в установщике нет лаунчера");
				using (FileStream f = File.Create(exe)) s.CopyTo(f);
			}
			string me = Assembly.GetExecutingAssembly().Location;
			string uninstall = Path.Combine(dir, "uninstall.exe");
			if (!string.Equals(Path.GetFullPath(me), Path.GetFullPath(uninstall), StringComparison.OrdinalIgnoreCase)) File.Copy(me, uninstall, true);
			if (desktop) Shortcut(DesktopLink, exe, dir);
			if (menu)
			{
				Directory.CreateDirectory(Path.GetDirectoryName(MenuLink));
				Shortcut(MenuLink, exe, dir);
			}
			using (RegistryKey k = Registry.CurrentUser.CreateSubKey(Key))
			{
				k.SetValue("DisplayName", "RayLink Launcher");
				k.SetValue("DisplayVersion", Version);
				k.SetValue("Publisher", "stormie-labs");
				k.SetValue("InstallLocation", dir);
				k.SetValue("DisplayIcon", exe);
				k.SetValue("UninstallString", "\"" + uninstall + "\" /uninstall");
				k.SetValue("QuietUninstallString", "\"" + uninstall + "\" /uninstall /S");
				k.SetValue("NoModify", 1, RegistryValueKind.DWord);
				k.SetValue("NoRepair", 1, RegistryValueKind.DWord);
				k.SetValue("EstimatedSize", 100, RegistryValueKind.DWord);
			}
			return exe;
		}

		static void Shortcut(string link, string target, string dir)
		{
			Type t = Type.GetTypeFromProgID("WScript.Shell");
			object sh = Activator.CreateInstance(t);
			object lnk = t.InvokeMember("CreateShortcut", BindingFlags.InvokeMethod, null, sh, new object[] { link });
			Type lt = lnk.GetType();
			lt.InvokeMember("TargetPath", BindingFlags.SetProperty, null, lnk, new object[] { target });
			lt.InvokeMember("WorkingDirectory", BindingFlags.SetProperty, null, lnk, new object[] { dir });
			lt.InvokeMember("IconLocation", BindingFlags.SetProperty, null, lnk, new object[] { target + ",0" });
			lt.InvokeMember("Description", BindingFlags.SetProperty, null, lnk, new object[] { "RayLink — кооператив для S.T.A.L.K.E.R. Anomaly и G.A.M.M.A" });
			lt.InvokeMember("Save", BindingFlags.InvokeMethod, null, lnk, null);
		}

		// The launcher, its shortcuts and the entry go; the player's settings (%APPDATA%\RayLink) and the stands stay.
		static int Uninstall(bool silent)
		{
			string dir = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
			if (!silent && MessageBox.Show("Удалить RayLink Launcher из " + dir + "?\nСтенды, сейвы и настройки останутся.", "RayLink",
				MessageBoxButtons.YesNo, MessageBoxIcon.Question) != DialogResult.Yes) return 1;
			if (Process.GetProcessesByName("RayLinkLauncher").Length > 0) throw new IOException("закрой RayLink Launcher и повтори");
			string exe = Path.Combine(dir, Exe);
			if (File.Exists(exe)) File.Delete(exe);
			foreach (string l in new[] { DesktopLink, MenuLink }) if (File.Exists(l)) File.Delete(l);
			string menuDir = Path.GetDirectoryName(MenuLink);
			if (Directory.Exists(menuDir) && Directory.GetFileSystemEntries(menuDir).Length == 0) Directory.Delete(menuDir);
			Registry.CurrentUser.DeleteSubKeyTree(Key, false);
			// this program cannot delete itself while it runs: cmd does it a moment later, then the folder if it is empty
			string me = Path.Combine(dir, "uninstall.exe");
			ProcessStartInfo psi = new ProcessStartInfo("cmd.exe", "/c ping 127.0.0.1 -n 3 > nul & del /f /q \"" + me + "\" & rmdir \"" + dir + "\"");
			psi.CreateNoWindow = true;
			psi.UseShellExecute = false;
			Process.Start(psi);
			if (!silent) MessageBox.Show("RayLink Launcher удалён.", "RayLink");
			return 0;
		}
	}

	class SetupForm : Form
	{
		static readonly Color Bg = Color.FromArgb(14, 15, 12), Panel2 = Color.FromArgb(26, 28, 24), Fg = Color.FromArgb(240, 241, 238),
			Dim = Color.FromArgb(160, 163, 156), Sand = Color.FromArgb(217, 180, 95);
		TextBox dir = new TextBox();
		CheckBox desktop = new CheckBox(), menu = new CheckBox(), run = new CheckBox();

		public SetupForm()
		{
			Text = "Установка RayLink";
			ClientSize = new Size(560, 300);
			FormBorderStyle = FormBorderStyle.FixedSingle;
			MaximizeBox = false;
			StartPosition = FormStartPosition.CenterScreen;
			BackColor = Bg;
			ForeColor = Fg;
			Font = new Font("Segoe UI", 10f);
			try { Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath); } catch { }
			Label title = new Label();
			title.Text = "RayLink";
			title.Font = new Font("Segoe UI", 22f, FontStyle.Bold);
			title.AutoSize = true;
			title.Location = new Point(24, 16);
			Controls.Add(title);
			Label sub = new Label();
			sub.Text = "Лаунчер кооператива S.T.A.L.K.E.R. Anomaly и G.A.M.M.A.\nСборку, скрипты и стенд он скачает и соберёт сам при первом запуске.";
			sub.ForeColor = Dim;
			sub.AutoSize = true;
			sub.Location = new Point(26, 64);
			Controls.Add(sub);
			Label where = new Label();
			where.Text = "Куда установить";
			where.ForeColor = Dim;
			where.AutoSize = true;
			where.Location = new Point(26, 116);
			Controls.Add(where);
			dir.Text = Path.Combine(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs"), "RayLink");
			dir.Location = new Point(26, 138);
			dir.Size = new Size(400, 26);
			dir.BackColor = Panel2;
			dir.ForeColor = Fg;
			dir.BorderStyle = BorderStyle.FixedSingle;
			Controls.Add(dir);
			Button browse = new Button();
			browse.Text = "Обзор…";
			browse.Location = new Point(436, 136);
			browse.Size = new Size(100, 30);
			browse.FlatStyle = FlatStyle.Flat;
			browse.BackColor = Panel2;
			browse.Click += delegate
			{
				using (FolderBrowserDialog d = new FolderBrowserDialog())
				{
					d.Description = "Папка для RayLink Launcher";
					if (d.ShowDialog(this) == DialogResult.OK) dir.Text = Path.Combine(d.SelectedPath, "RayLink");
				}
			};
			Controls.Add(browse);
			int y = 178;
			foreach (CheckBox c in new[] { desktop, menu, run })
			{
				c.ForeColor = Fg;
				c.AutoSize = true;
				c.Checked = true;
				c.Location = new Point(26, y);
				Controls.Add(c);
				y += 26;
			}
			desktop.Text = "Ярлык на рабочем столе";
			menu.Text = "Ярлык в меню «Пуск»";
			run.Text = "Запустить RayLink после установки";
			Button install = new Button();
			install.Text = "Установить";
			install.Font = new Font("Segoe UI", 12f, FontStyle.Bold);
			install.Location = new Point(386, 244);
			install.Size = new Size(150, 40);
			install.FlatStyle = FlatStyle.Flat;
			install.FlatAppearance.BorderColor = Sand;
			install.BackColor = Sand;
			install.ForeColor = Color.FromArgb(20, 18, 12);
			install.Click += delegate
			{
				try
				{
					string exe = Setup.Install(dir.Text.Trim(), desktop.Checked, menu.Checked);
					if (run.Checked)
					{
						ProcessStartInfo psi = new ProcessStartInfo(exe);
						psi.WorkingDirectory = Path.GetDirectoryName(exe);
						Process.Start(psi);
					}
					else MessageBox.Show(this, "RayLink Launcher установлен в " + Path.GetDirectoryName(exe), "RayLink");
					Close();
				}
				catch (Exception e)
				{
					MessageBox.Show(this, "Не удалось установить: " + e.Message, "RayLink", MessageBoxButtons.OK, MessageBoxIcon.Error);
				}
			};
			Controls.Add(install);
		}
	}
}
