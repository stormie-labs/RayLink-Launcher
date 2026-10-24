// RayLink Launcher v0.1 ("lite"): one window for the co-op test stand. C# 5 / .NET Framework 4.8 (built into Windows 10/11),
// built by lite\build.cmd with the framework's own csc.exe: nothing to install on the player's PC.
// The v1 launcher of docs\SPEC.md (Tauri) replaces it later; the rules here follow the spec where they exist:
//  - the game: G.A.M.M.A (an MO2 profile over Anomaly) or plain Anomaly, chosen at the top left (SPEC 2.1);
//  - the stand: a folder of hard links, the way MO2 shows the game (anomaly-coop\tools\mo2_stand.py): the game folder,
//    our engine layer, the enabled mods in the profile's priority, MO2's overwrite, our mod layer; files the game writes
//    and our own layers are copied, nothing is ever written into a linked file, the originals are never changed;
//  - a build: the latest release of the RayLink repository (the MO2 archive of tools\package.ps1);
//  - play: bin\ + fsgame_a.ltx (host or client), bin_b\ + fsgame_b.ltx for a second window on this PC.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Net;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// .NET 4.8 semantics (long paths: GAMMA has files deeper than 260 characters)
[assembly: System.Runtime.Versioning.TargetFramework(".NETFramework,Version=v4.8", FrameworkDisplayName = ".NET Framework 4.8")]
[assembly: System.Reflection.AssemblyTitle("RayLink Launcher")]
[assembly: System.Reflection.AssemblyProduct("RayLink")]
[assembly: System.Reflection.AssemblyVersion("0.1.1.0")]
[assembly: System.Reflection.AssemblyFileVersion("0.1.1.0")]

namespace RayLink
{
	static class Program
	{
		public const string Version = "0.1.1";
		public const string DefaultRepo = "REPLACE_OWNER/RayLink"; // set by build.cmd (/define is not enough for strings)
		public const string ExeName = "AnomalyCoopDX11AVX.exe";

		[STAThread]
		static void Main(string[] args)
		{
			ServicePointManager.SecurityProtocol = (SecurityProtocolType)3072; // TLS 1.2 for GitHub
			// Self-test without the window: --stand <build.zip> installs that archive as build "local" and builds the stand
			// of the saved settings (the log goes to %APPDATA%\RayLink\launcher.log and stdout).
			if (args.Length == 2 && args[0] == "--stand")
			{
				try { MainForm.Headless(args[1]); }
				catch (Exception e) { Console.WriteLine("! " + e.Message); Environment.Exit(1); }
				return;
			}
			Application.EnableVisualStyles();
			Application.SetCompatibleTextRenderingDefault(false);
			Application.Run(new MainForm());
		}
	}

	// ------------------------------------------------------------------ settings (%APPDATA%\RayLink\settings.txt)
	class Settings
	{
		public string Game = "gamma"; // gamma | anomaly
		public string AnomalyDir = "";
		public string Mo2Dir = "";
		public string Profile = "";
		public string StandRoot = "";
		public string Nick = "";
		public string Repo = Program.DefaultRepo;
		public string Address = "";
		public string Installed = ""; // release tag in builds\
		public string TokenProtected = ""; // DPAPI, current user
		public bool Join = false;
		public bool SecondWindow = false;

		public static string Dir
		{
			get { return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "RayLink"); }
		}
		static string FileName { get { return Path.Combine(Dir, "settings.txt"); } }

		public static Settings Load()
		{
			Settings s = new Settings();
			try
			{
				if (!File.Exists(FileName)) return s;
				foreach (string line in File.ReadAllLines(FileName, Encoding.UTF8))
				{
					int eq = line.IndexOf('=');
					if (eq <= 0) continue;
					string k = line.Substring(0, eq).Trim(), v = line.Substring(eq + 1).Trim();
					switch (k)
					{
						case "game": s.Game = v == "anomaly" ? "anomaly" : "gamma"; break;
						case "anomaly": s.AnomalyDir = v; break;
						case "mo2": s.Mo2Dir = v; break;
						case "profile": s.Profile = v; break;
						case "stand": s.StandRoot = v; break;
						case "nick": s.Nick = v; break;
						case "repo": if (v.Length > 0 && v != "stormie-labs/RayLink-Engine") s.Repo = v; break; // builds moved to RayLink-Coop
						case "address": s.Address = v; break;
						case "installed": s.Installed = v; break;
						case "token": s.TokenProtected = v; break;
						case "join": s.Join = v == "1"; break;
						case "second": s.SecondWindow = v == "1"; break;
					}
				}
			}
			catch { }
			return s;
		}

		public void Save()
		{
			Directory.CreateDirectory(Dir);
			string[] lines = {
				"game=" + Game, "anomaly=" + AnomalyDir, "mo2=" + Mo2Dir, "profile=" + Profile, "stand=" + StandRoot,
				"nick=" + Nick, "repo=" + Repo, "address=" + Address, "installed=" + Installed, "token=" + TokenProtected,
				"join=" + (Join ? "1" : "0"), "second=" + (SecondWindow ? "1" : "0") };
			string tmp = FileName + ".tmp";
			File.WriteAllLines(tmp, lines, Encoding.UTF8);
			if (File.Exists(FileName)) File.Delete(FileName);
			File.Move(tmp, FileName);
		}

		public string Token
		{
			get
			{
				if (TokenProtected.Length == 0) return "";
				try
				{
					byte[] raw = ProtectedData.Unprotect(Convert.FromBase64String(TokenProtected), null, DataProtectionScope.CurrentUser);
					return Encoding.UTF8.GetString(raw);
				}
				catch { return ""; }
			}
			set
			{
				TokenProtected = string.IsNullOrEmpty(value) ? "" : Convert.ToBase64String(
					ProtectedData.Protect(Encoding.UTF8.GetBytes(value), null, DataProtectionScope.CurrentUser));
			}
		}
	}

	// ------------------------------------------------------------------ the game folders
	static class Games
	{
		public static string CheckAnomaly(string dir)
		{
			if (string.IsNullOrEmpty(dir) || !Directory.Exists(dir)) return "папка не найдена";
			foreach (string need in new[] { "db", "gamedata", "bin" })
				if (!Directory.Exists(Path.Combine(dir, need))) return "нет папки " + need + "\\ — это не Anomaly";
			if (!File.Exists(Path.Combine(dir, "fsgame.ltx"))) return "нет fsgame.ltx — это не Anomaly";
			return null;
		}

		public static string CheckMo2(string dir)
		{
			if (string.IsNullOrEmpty(dir) || !Directory.Exists(dir)) return "папка не найдена";
			if (!File.Exists(Path.Combine(dir, "ModOrganizer.ini"))) return "нет ModOrganizer.ini — это не папка G.A.M.M.A (MO2)";
			if (Profiles(dir).Count == 0) return "в MO2 нет профилей";
			return null;
		}

		static string Ini(string mo2, string key)
		{
			try
			{
				foreach (string line in File.ReadAllLines(Path.Combine(mo2, "ModOrganizer.ini")))
				{
					if (!line.StartsWith(key + "=", StringComparison.OrdinalIgnoreCase)) continue;
					string v = line.Substring(key.Length + 1).Trim();
					if (v.StartsWith("@ByteArray(") && v.EndsWith(")")) v = v.Substring(11, v.Length - 12);
					return v.Replace("/", "\\").Replace("\\\\", "\\");
				}
			}
			catch { }
			return "";
		}

		public static string GamePath(string mo2) { return Ini(mo2, "gamePath"); }

		static string Mo2Folder(string mo2, string key, string def)
		{
			string v = Ini(mo2, key);
			if (v.Length == 0) return Path.Combine(mo2, def);
			v = v.Replace("%BASE_DIR%", mo2);
			return Path.IsPathRooted(v) ? v : Path.Combine(mo2, v);
		}

		public static string ModsDir(string mo2) { return Mo2Folder(mo2, "mod_directory", "mods"); }
		public static string OverwriteDir(string mo2) { return Mo2Folder(mo2, "overwrite_directory", "overwrite"); }
		public static string ProfilesDir(string mo2) { return Mo2Folder(mo2, "profiles_directory", "profiles"); }

		public static List<string> Profiles(string mo2)
		{
			List<string> list = new List<string>();
			try
			{
				string pd = ProfilesDir(mo2);
				if (Directory.Exists(pd))
					foreach (string d in Directory.GetDirectories(pd))
						if (File.Exists(Path.Combine(d, "modlist.txt"))) list.Add(Path.GetFileName(d));
			}
			catch { }
			return list;
		}

		public static string SelectedProfile(string mo2)
		{
			string p = Ini(mo2, "selected_profile");
			return p;
		}

		public static string GammaVersion(string mo2)
		{
			try
			{
				string f = Path.Combine(mo2, ".Grok's Modpack Installer", "G.A.M.M.A_definition_version.txt");
				if (File.Exists(f)) return File.ReadAllText(f).Trim();
			}
			catch { }
			return "";
		}

		// Enabled mods of an MO2 modlist.txt, lowest priority first, without separators (MO2 writes the highest first).
		public static List<string> ReadModlist(string path)
		{
			List<string> mods = new List<string>();
			foreach (string raw in File.ReadAllLines(path, Encoding.UTF8))
			{
				string line = raw.TrimStart('﻿');
				if (!line.StartsWith("+")) continue;
				string name = line.Substring(1).Trim();
				if (name.Length > 0 && !name.EndsWith("_separator")) mods.Add(name);
			}
			mods.Reverse();
			return mods;
		}

		public static string Find(string[] names, Func<string, string> check)
		{
			foreach (DriveInfo d in DriveInfo.GetDrives())
			{
				try
				{
					if (d.DriveType != DriveType.Fixed || !d.IsReady) continue;
					foreach (string n in names)
					{
						foreach (string where in new[] { "", "Games", "Program Files", "Program Files (x86)" })
						{
							string p = Path.Combine(Path.Combine(d.RootDirectory.FullName, where), n);
							if (check(p) == null) return p;
						}
					}
				}
				catch { }
			}
			return "";
		}
	}

	// ------------------------------------------------------------------ the stand (port of tools\mo2_stand.py)
	class Layer
	{
		public string Name, Root, Prefix = "", Kind = "mod"; // game | mod | overwrite | ours
		public bool Copy;
	}

	static class Native
	{
		// The Win32 calls below get the extended-length form: some G.A.M.M.A files lie deeper than MAX_PATH.
		public static string Long(string path)
		{
			string full = Path.GetFullPath(path);
			return full.StartsWith(@"\\?\") ? full : @"\\?\" + full;
		}

		public static bool HardLink(string dst, string src) { return CreateHardLink(Long(dst), Long(src), IntPtr.Zero); }

		[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
		public static extern bool CreateHardLink(string lpFileName, string lpExistingFileName, IntPtr lpSecurityAttributes);

		[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
		static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disposition, uint flags, IntPtr template);

		[DllImport("kernel32.dll", SetLastError = true)]
		static extern bool SetFileInformationByHandle(IntPtr h, int cls, ref uint info, uint size);

		[DllImport("kernel32.dll", SetLastError = true)]
		static extern bool CloseHandle(IntPtr h);

		// Delete one name of a read-only file without touching the attribute, which a hard link shares with the mod's
		// original (FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, Windows 10 1809+).
		public static void DeleteIgnoringReadonly(string path)
		{
			IntPtr h = CreateFileW(Long(path), 0x00010000, 7, IntPtr.Zero, 3, 0x00200000, IntPtr.Zero);
			if (h == new IntPtr(-1)) throw new IOException("cannot open for delete: " + path + " (" + Marshal.GetLastWin32Error() + ")");
			try
			{
				uint flags = 0x1 | 0x2 | 0x10;
				if (!SetFileInformationByHandle(h, 21, ref flags, 4))
					throw new IOException("cannot delete: " + path + " (" + Marshal.GetLastWin32Error() + ")");
			}
			finally { CloseHandle(h); }
		}
	}

	class Stand
	{
		public const string Manifest = ".raylink_stand.tsv";
		Action<string> log;
		public Stand(Action<string> log) { this.log = log; }

		// Places the game and the mods' scripts write into while running: the stand's root, appdata\, gamedata\ root,
		// gamedata\configs\ root and the text files next to the exe (tools\mo2_stand.py runtime_written).
		static bool RuntimeWritten(string rel)
		{
			string[] parts = rel.ToLowerInvariant().Split('/');
			if (parts.Length == 1 || parts[0] == "appdata") return true;
			if (parts[0] == "bin" && parts.Length == 2 &&
				(parts[1].EndsWith(".ini") || parts[1].EndsWith(".log") || parts[1].EndsWith(".txt"))) return true;
			return parts[0] == "gamedata" && (parts.Length == 2 || (parts.Length == 3 && parts[1] == "configs"));
		}

		class Entry { public string Rel, Src, Mode; public long Size, Mtime; }

		static IEnumerable<Entry> LayerFiles(Layer layer)
		{
			Stack<KeyValuePair<string, string>> stack = new Stack<KeyValuePair<string, string>>();
			stack.Push(new KeyValuePair<string, string>(layer.Root, ""));
			while (stack.Count > 0)
			{
				KeyValuePair<string, string> top = stack.Pop();
				DirectoryInfo di = new DirectoryInfo(top.Key);
				foreach (DirectoryInfo sub in di.GetDirectories())
				{
					if ((sub.Attributes & FileAttributes.ReparsePoint) != 0) continue;
					string low = sub.Name.ToLowerInvariant();
					if (layer.Kind == "game" && top.Value.Length == 0 && low == "appdata") continue;
					if (low == ".git") continue;
					stack.Push(new KeyValuePair<string, string>(sub.FullName, top.Value.Length == 0 ? sub.Name : top.Value + "/" + sub.Name));
				}
				foreach (FileInfo f in di.GetFiles())
				{
					string low = f.Name.ToLowerInvariant();
					if (low.EndsWith(".mohidden")) continue;
					if (top.Value.Length == 0 && low == "meta.ini" && layer.Kind == "mod") continue;
					string rel = top.Value.Length == 0 ? f.Name : top.Value + "/" + f.Name;
					if (layer.Prefix.Length > 0) rel = layer.Prefix + "/" + rel;
					Entry e = new Entry();
					e.Rel = rel; e.Src = f.FullName; e.Size = f.Length; e.Mtime = f.LastWriteTimeUtc.Ticks;
					yield return e;
				}
			}
		}

		static bool Inside(string path, string folder)
		{
			path = Path.GetFullPath(path).TrimEnd('\\').ToLowerInvariant();
			folder = Path.GetFullPath(folder).TrimEnd('\\').ToLowerInvariant();
			return path == folder || path.StartsWith(folder + "\\");
		}

		static void Unlink(string path)
		{
			try { File.Delete(path); }
			catch (UnauthorizedAccessException e)
			{
				if ((File.GetAttributes(path) & FileAttributes.ReadOnly) == 0)
					throw new IOException("нет доступа к " + path + ": файл занят — игра (или её второе окно, или окно вылета) " +
						"ещё запущена из стенда? (" + e.Message + ")");
				Native.DeleteIgnoringReadonly(path);
			}
		}

		// Processes started from inside the stand (the game, its second window, a crash window that keeps it alive): Windows
		// refuses to delete a running exe ("access denied"), so the stand cannot be rebuilt under them. Live 2026-10-24: the
		// owner's laptop failed the build 16 -> 17 update with "access denied".
		public static string RunningFrom(string dir)
		{
			List<string> found = new List<string>();
			foreach (Process p in Process.GetProcesses())
			{
				try
				{
					if (Inside(p.MainModule.FileName, dir)) found.Add(p.ProcessName + " (PID " + p.Id + ")");
				}
				catch (Exception) { } // system and other users' processes: no access, and not ours
				finally { p.Dispose(); }
			}
			return string.Join(", ", found.ToArray());
		}

		static void Place(string src, string dst, bool copy)
		{
			if (Directory.Exists(dst)) throw new IOException("на месте файла стоит папка: " + dst);
			if (File.Exists(dst)) Unlink(dst); // never write into an existing file: it may be a link to a mod's original
			if (copy)
			{
				File.Copy(src, dst);
				FileAttributes a = File.GetAttributes(dst);
				if ((a & FileAttributes.ReadOnly) != 0) File.SetAttributes(dst, a & ~FileAttributes.ReadOnly);
				return;
			}
			if (!Native.HardLink(dst, src))
				throw new IOException("жёсткая ссылка не создалась: " + src + " -> " + dst + " (" + Marshal.GetLastWin32Error() +
					"). Стенд должен лежать на том же диске, что игра и моды.");
		}

		static void EnsureDir(string outDir, string relDir, HashSet<string> ok)
		{
			string cur = outDir;
			if (relDir.Length == 0) return;
			foreach (string part in relDir.Split('/'))
			{
				cur = Path.Combine(cur, part);
				string key = cur.ToLowerInvariant();
				if (ok.Contains(key)) continue;
				if (Directory.Exists(cur))
				{
					if ((new DirectoryInfo(cur).Attributes & FileAttributes.ReparsePoint) != 0)
						throw new IOException("через ссылку на папку не пишу: " + cur);
				}
				else if (File.Exists(cur)) throw new IOException("на месте папки стоит файл: " + cur);
				else Directory.CreateDirectory(cur);
				ok.Add(key);
			}
		}

		public void Materialize(List<Layer> layers, string outDir)
		{
			Stopwatch sw = Stopwatch.StartNew();
			foreach (Layer l in layers)
				if (Inside(outDir, l.Root) || Inside(l.Root, outDir))
					throw new IOException("папка стенда пересекается со слоем " + l.Name + " (" + l.Root + ")");
			Dictionary<string, Entry> want = new Dictionary<string, Entry>();
			int mods = 0;
			List<string> missing = new List<string>();
			foreach (Layer l in layers)
			{
				if (!Directory.Exists(l.Root))
				{
					if (l.Kind == "mod") { missing.Add(l.Name); continue; }
					if (l.Kind == "overwrite") continue;
					throw new IOException("нет папки слоя: " + l.Root);
				}
				if (l.Kind == "mod") mods++;
				foreach (Entry e in LayerFiles(l))
				{
					e.Mode = (l.Copy || RuntimeWritten(e.Rel)) ? "copy" : "link";
					want[e.Rel.ToLowerInvariant()] = e;
				}
			}
			log(string.Format("стенд: {0} файлов из {1} слоёв ({2} модов){3}", want.Count, layers.Count, mods,
				missing.Count > 0 ? ", нет папок у " + missing.Count + " модов" : ""));

			string manifestPath = Path.Combine(outDir, Manifest);
			Dictionary<string, Entry> old = new Dictionary<string, Entry>();
			if (File.Exists(manifestPath))
			{
				foreach (string line in File.ReadAllLines(manifestPath, Encoding.UTF8))
				{
					string[] p = line.Split('\t');
					if (p.Length < 6) continue;
					Entry e = new Entry();
					e.Rel = p[1]; e.Src = p[2]; e.Size = long.Parse(p[3]); e.Mtime = long.Parse(p[4]); e.Mode = p[5];
					old[p[0]] = e;
				}
			}
			else if (Directory.Exists(outDir) && Directory.GetFileSystemEntries(outDir).Length > 0)
				throw new IOException(outDir + " не пустая и это не стенд RayLink — выбери пустую папку");
			Directory.CreateDirectory(outDir);

			HashSet<string> okDirs = new HashSet<string>();
			foreach (Entry e in want.Values)
			{
				int slash = e.Rel.LastIndexOf('/');
				EnsureDir(outDir, slash < 0 ? "" : e.Rel.Substring(0, slash), okDirs);
			}

			Dictionary<string, Entry> state = new Dictionary<string, Entry>(old);
			int removed = 0, linked = 0, copied = 0, same = 0;
			try
			{
				foreach (KeyValuePair<string, Entry> kv in old)
				{
					if (want.ContainsKey(kv.Key)) continue;
					string dst = Path.Combine(outDir, kv.Value.Rel.Replace('/', '\\'));
					if (!Inside(dst, outDir)) throw new IOException("удаление вне стенда: " + dst);
					if (File.Exists(dst)) Unlink(dst);
					state.Remove(kv.Key);
					removed++;
				}
				foreach (KeyValuePair<string, Entry> kv in want)
				{
					Entry e = kv.Value;
					string dst = Path.Combine(outDir, e.Rel.Replace('/', '\\'));
					Entry prev;
					if (old.TryGetValue(kv.Key, out prev) && prev.Src == e.Src && prev.Size == e.Size && prev.Mtime == e.Mtime &&
						prev.Mode == e.Mode && File.Exists(dst))
					{
						same++;
						continue;
					}
					Place(e.Src, dst, e.Mode == "copy");
					state[kv.Key] = e;
					if (e.Mode == "copy") copied++; else linked++;
					if ((linked + copied) % 5000 == 0) log("  … " + (linked + copied) + " положено");
				}
			}
			finally
			{
				StringBuilder sb = new StringBuilder();
				foreach (KeyValuePair<string, Entry> kv in state)
					sb.Append(kv.Key).Append('\t').Append(kv.Value.Rel).Append('\t').Append(kv.Value.Src).Append('\t')
						.Append(kv.Value.Size).Append('\t').Append(kv.Value.Mtime).Append('\t').Append(kv.Value.Mode).Append('\n');
				string tmp = manifestPath + ".tmp";
				File.WriteAllText(tmp, sb.ToString(), new UTF8Encoding(false));
				if (File.Exists(manifestPath)) File.Delete(manifestPath);
				File.Move(tmp, manifestPath);
			}
			log(string.Format("стенд готов за {0:F1} с: ссылок {1}, копий {2}, без изменений {3}, убрано {4}",
				sw.Elapsed.TotalSeconds, linked, copied, same, removed));
		}

		static readonly Encoding Cp1251 = Encoding.GetEncoding(1251);

		// bin_b (a second window: the engine refuses a second instance from the same exe folder), fsgame_a/_b.ltx and
		// appdata_a/_b with user.ltx (tools\deploy.ps1).
		public void Instances(string outDir, string anomalyDir, string nick)
		{
			string binA = Path.Combine(outDir, "bin"), binB = Path.Combine(outDir, "bin_b");
			if (Directory.Exists(binB))
			{
				if ((new DirectoryInfo(binB).Attributes & FileAttributes.ReparsePoint) != 0) throw new IOException("bin_b — ссылка: " + binB);
				foreach (string f in Directory.GetFiles(binB, "*", SearchOption.AllDirectories)) Unlink(f);
			}
			foreach (string f in Directory.GetFiles(binA, "*", SearchOption.AllDirectories))
			{
				string dst = Path.Combine(binB, f.Substring(binA.Length + 1));
				Directory.CreateDirectory(Path.GetDirectoryName(dst));
				if (!Native.HardLink(dst, f)) File.Copy(f, dst, true);
			}
			string fs = File.ReadAllText(Path.Combine(outDir, "fsgame.ltx"), Cp1251);
			foreach (string inst in new[] { "a", "b" })
			{
				string name = "appdata_" + inst;
				string text = Regex.Replace(fs, @"(?m)^(\$app_data_root\$\s*=.*\|\s*)appdata\\", "${1}" + name + "\\");
				if (text.IndexOf(name + "\\", StringComparison.Ordinal) < 0) throw new IOException("fsgame.ltx непонятного вида: app_data_root не переписан");
				File.WriteAllText(Path.Combine(outDir, "fsgame_" + inst + ".ltx"), text, Cp1251);
				string ad = Path.Combine(outDir, name);
				Directory.CreateDirectory(ad);
				string userOut = Path.Combine(ad, "user.ltx");
				if (!File.Exists(userOut))
				{
					List<string> lines = new List<string>();
					string userIn = Path.Combine(Path.Combine(anomalyDir, "appdata"), "user.ltx");
					if (File.Exists(userIn)) lines.AddRange(File.ReadAllLines(userIn, Cp1251));
					Dictionary<string, string> set = new Dictionary<string, string>();
					if (inst == "b")
					{
						set["rs_screenmode"] = "windowed";
						set["vid_mode"] = "1280x720";
					}
					set["g_always_active"] = "on";
					foreach (KeyValuePair<string, string> kv in set)
					{
						int i = lines.FindIndex(l => Regex.IsMatch(l, "^" + kv.Key + @"\s"));
						if (i >= 0) lines[i] = kv.Key + " " + kv.Value; else lines.Add(kv.Key + " " + kv.Value);
					}
					lines.Add("coop_nick " + (inst == "a" ? nick : nick + "_2"));
					File.WriteAllLines(userOut, lines.ToArray(), Cp1251);
				}
				string modsAppdata = Path.Combine(outDir, "appdata");
				if (Directory.Exists(modsAppdata))
					foreach (string f in Directory.GetFiles(modsAppdata, "*", SearchOption.AllDirectories))
					{
						if (Path.GetFileName(f).ToLowerInvariant() == "user.ltx") continue;
						string dst = Path.Combine(ad, f.Substring(modsAppdata.Length + 1));
						if (File.Exists(dst)) continue;
						Directory.CreateDirectory(Path.GetDirectoryName(dst));
						File.Copy(f, dst);
					}
			}
			log("окна: bin + fsgame_a.ltx (ты), bin_b + fsgame_b.ltx (второе окно)");
		}
	}

	// ------------------------------------------------------------------ the build channel (GitHub releases)
	class Release
	{
		public string Tag = "", Name = "", Body = "", AssetName = "", AssetApiUrl = "", AssetUrl = "";
		public long AssetSize;
	}

	static class Channel
	{
		static WebClient Client(string token, string accept)
		{
			WebClient wc = new WebClient();
			wc.Headers[HttpRequestHeader.UserAgent] = "RayLink-Launcher/" + Program.Version;
			wc.Headers[HttpRequestHeader.Accept] = accept;
			if (!string.IsNullOrEmpty(token)) wc.Headers[HttpRequestHeader.Authorization] = "token " + token;
			return wc;
		}

		public static Release Latest(string repo, string token)
		{
			string json;
			using (WebClient wc = Client(token, "application/vnd.github+json"))
				json = wc.DownloadString("https://api.github.com/repos/" + repo + "/releases/latest");
			Dictionary<string, object> r = (Dictionary<string, object>)new JavaScriptSerializer().DeserializeObject(json);
			Release rel = new Release();
			rel.Tag = Convert.ToString(r["tag_name"]);
			rel.Name = r.ContainsKey("name") ? Convert.ToString(r["name"]) : rel.Tag;
			rel.Body = r.ContainsKey("body") ? Convert.ToString(r["body"]) : "";
			object[] assets = r.ContainsKey("assets") ? (object[])r["assets"] : new object[0];
			foreach (object o in assets)
			{
				Dictionary<string, object> a = (Dictionary<string, object>)o;
				string name = Convert.ToString(a["name"]);
				if (!name.EndsWith(".zip", StringComparison.OrdinalIgnoreCase)) continue;
				rel.AssetName = name;
				rel.AssetApiUrl = Convert.ToString(a["url"]);
				rel.AssetUrl = Convert.ToString(a["browser_download_url"]);
				rel.AssetSize = Convert.ToInt64(a["size"]);
				break;
			}
			if (rel.AssetName.Length == 0) throw new IOException("в релизе " + rel.Tag + " нет zip-архива сборки");
			return rel;
		}

		public static void Download(Release rel, string token, string file)
		{
			string tmp = file + ".part";
			if (File.Exists(tmp)) File.Delete(tmp);
			if (string.IsNullOrEmpty(token))
			{
				using (WebClient wc = Client(null, "application/octet-stream")) wc.DownloadFile(rel.AssetUrl, tmp);
			}
			else
			{
				using (WebClient wc = Client(token, "application/octet-stream")) wc.DownloadFile(rel.AssetApiUrl, tmp);
			}
			if (rel.AssetSize > 0 && new FileInfo(tmp).Length != rel.AssetSize)
				throw new IOException("архив скачался не целиком: " + new FileInfo(tmp).Length + " из " + rel.AssetSize + " байт");
			if (File.Exists(file)) File.Delete(file);
			File.Move(tmp, file);
		}
	}

	// ------------------------------------------------------------------ the window
	class MainForm : Form
	{
		static readonly Color Bg = Color.FromArgb(14, 15, 12), Panel2 = Color.FromArgb(26, 28, 24), Fg = Color.FromArgb(240, 241, 238),
			Dim = Color.FromArgb(160, 163, 156), Sand = Color.FromArgb(217, 180, 95), Sage = Color.FromArgb(150, 178, 140),
			Bad = Color.FromArgb(214, 106, 92);

		Settings S = Settings.Load();
		ComboBox cbGame = new ComboBox(), cbProfile = new ComboBox();
		Label lblBuild = new Label(), lblBuildState = new Label(), lblStand = new Label(), lblGame = new Label();
		Button btnUpdate = new Button(), btnStand = new Button(), btnPlay = new Button(), btnSettings = new Button(), btnOpen = new Button();
		RadioButton rbHost = new RadioButton(), rbJoin = new RadioButton();
		TextBox tbAddress = new TextBox(), tbNick = new TextBox(), tbLog = new TextBox();
		CheckBox chkSecond = new CheckBox();
		bool busy, loading;
		int prevGame;

		public static void Headless(string zip)
		{
			MainForm f = new MainForm();
			f.headless = true;
			string dir = f.BuildDir("local");
			if (Directory.Exists(dir)) Directory.Delete(dir, true);
			ZipFile.ExtractToDirectory(zip, dir);
			f.S.Installed = "local";
			f.BuildStand();
		}

		bool headless;

		public MainForm()
		{
			Text = "RayLink Launcher " + Program.Version;
			ClientSize = new Size(1040, 640);
			FormBorderStyle = FormBorderStyle.FixedSingle;
			MaximizeBox = false;
			StartPosition = FormStartPosition.CenterScreen;
			BackColor = Bg;
			ForeColor = Fg;
			Font = new Font("Segoe UI", 10f);
			try { Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath); } catch { }

			Label title = Lbl("RayLink", 20, 14, 16f, FontStyle.Bold, Fg);
			title.AutoSize = true;
			Controls.Add(title);
			cbGame.DropDownStyle = ComboBoxStyle.DropDownList;
			cbGame.Items.AddRange(new object[] { "G.A.M.M.A", "Anomaly (без модов)" });
			Place(cbGame, 140, 20, 190, 28);
			StyleInput(cbGame);
			cbProfile.DropDownStyle = ComboBoxStyle.DropDownList;
			Place(cbProfile, 340, 20, 190, 28);
			StyleInput(cbProfile);
			lblGame.ForeColor = Dim;
			Place(lblGame, 540, 24, 480, 22);

			Controls.Add(Lbl("СБОРКА", 40, 92, 10f, FontStyle.Bold, Dim));
			lblBuild.Font = new Font("Segoe UI", 54f, FontStyle.Bold);
			lblBuild.ForeColor = Fg;
			Place(lblBuild, 32, 110, 380, 96);
			lblBuildState.ForeColor = Dim;
			Place(lblBuildState, 40, 210, 420, 44);

			Btn(btnUpdate, "Обновить сборку", 40, 262, 200, 36, false);
			Btn(btnStand, "Собрать стенд", 250, 262, 200, 36, false);

			Controls.Add(Lbl("Ник", 40, 322, 10f, FontStyle.Regular, Dim));
			Place(tbNick, 40, 344, 200, 26);
			StyleInput(tbNick);
			// Owner 2026-10-24: a launcher for every player - hosting and joining are in the game's own Co-op window.
			Controls.Add(Lbl("Создать игру или подключиться — в игре, окно «Кооператив»", 40, 384, 9.5f, FontStyle.Regular, Dim));

			Btn(btnPlay, "ИГРАТЬ", 40, 420, 300, 72, true);

			Controls.Add(Lbl("Что происходит", 500, 92, 10f, FontStyle.Bold, Dim));
			tbLog.Multiline = true;
			tbLog.ReadOnly = true;
			tbLog.ScrollBars = ScrollBars.Vertical;
			tbLog.BackColor = Panel2;
			tbLog.ForeColor = Fg;
			tbLog.BorderStyle = BorderStyle.None;
			tbLog.Font = new Font("Consolas", 9f);
			Place(tbLog, 500, 116, 510, 476);

			lblStand.ForeColor = Dim;
			Place(lblStand, 20, 608, 620, 22);
			Btn(btnOpen, "Открыть папку", 650, 602, 170, 30, false);
			Btn(btnSettings, "Настройки", 830, 602, 180, 30, false);

			loading = true;
			cbGame.SelectedIndex = S.Game == "anomaly" ? 1 : 0;
			prevGame = cbGame.SelectedIndex;
			tbNick.Text = S.Nick.Length > 0 ? S.Nick : CleanNick(Environment.UserName);
			tbAddress.Text = S.Address;
			rbJoin.Checked = S.Join;
			rbHost.Checked = !S.Join;
			chkSecond.Checked = S.SecondWindow;
			loading = false;

			cbGame.SelectedIndexChanged += delegate { OnGameChanged(); };
			cbProfile.SelectedIndexChanged += delegate { if (!loading) { S.Profile = Convert.ToString(cbProfile.SelectedItem); S.Save(); Refresh2(); } };
			rbJoin.CheckedChanged += delegate { tbAddress.Enabled = rbJoin.Checked; };
			btnUpdate.Click += delegate { Run("обновление", UpdateBuild); };
			btnStand.Click += delegate { Run("сборка стенда", BuildStand); };
			btnPlay.Click += delegate { Play(); };
			btnOpen.Click += delegate { string d = StandDir(); if (Directory.Exists(d)) Process.Start("explorer.exe", d); };
			btnSettings.Click += delegate { ShowSettings(); };
			Shown += delegate { Startup(); };
		}

		// ---------------------------------------------------------- small UI helpers
		Label Lbl(string text, int x, int y, float size, FontStyle st, Color c)
		{
			Label l = new Label();
			l.Text = text;
			l.Font = new Font("Segoe UI", size, st);
			l.ForeColor = c;
			l.AutoSize = true;
			l.Location = new Point(x, y);
			l.BackColor = Color.Transparent;
			return l;
		}

		void Place(Control c, int x, int y, int w, int h)
		{
			c.Location = new Point(x, y);
			c.Size = new Size(w, h);
			Controls.Add(c);
		}

		void StyleInput(Control c)
		{
			c.BackColor = Panel2;
			c.ForeColor = Fg;
			TextBox tb = c as TextBox;
			if (tb != null) tb.BorderStyle = BorderStyle.FixedSingle;
			ComboBox cb = c as ComboBox;
			if (cb != null) cb.FlatStyle = FlatStyle.Flat;
		}

		void Btn(Button b, string text, int x, int y, int w, int h, bool main)
		{
			b.Text = text;
			b.FlatStyle = FlatStyle.Flat;
			b.FlatAppearance.BorderColor = main ? Sand : Color.FromArgb(70, 72, 66);
			b.BackColor = main ? Sand : Panel2;
			b.ForeColor = main ? Color.FromArgb(20, 18, 12) : Fg;
			if (main) b.Font = new Font("Segoe UI", 20f, FontStyle.Bold);
			b.Cursor = Cursors.Hand;
			Place(b, x, y, w, h);
		}

		void Log(string line)
		{
			if (headless) Console.WriteLine(line);
			if (InvokeRequired && !headless) { BeginInvoke(new Action<string>(Log), line); return; }
			tbLog.AppendText(DateTime.Now.ToString("HH:mm:ss") + "  " + line + "\r\n");
			try { File.AppendAllText(Path.Combine(Settings.Dir, "launcher.log"), DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss") + "  " + line + "\r\n"); } catch { }
		}

		static string CleanNick(string s)
		{
			string n = Regex.Replace(s ?? "", "[^A-Za-z0-9_.-]", "");
			if (n.Length > 24) n = n.Substring(0, 24);
			return n.Length >= 2 ? n : "stalker";
		}

		bool Gamma { get { return cbGame.SelectedIndex == 0; } }

		string StandDir()
		{
			if (string.IsNullOrEmpty(S.StandRoot)) return "";
			return Path.Combine(S.StandRoot, Gamma ? "gamma" : "anomaly");
		}

		string BuildDir(string tag) { return Path.Combine(Path.Combine(Application.StartupPath, "builds"), tag); } // the home

		// ---------------------------------------------------------- the game and its paths (SPEC 2.1)
		void Startup()
		{
			Log("RayLink Launcher " + Program.Version + ", канал " + S.Repo);
			if (!EnsurePaths(Gamma))
			{
				if (Gamma && Games.CheckAnomaly(S.AnomalyDir) == null)
				{
					loading = true;
					cbGame.SelectedIndex = 1;
					loading = false;
				}
			}
			prevGame = cbGame.SelectedIndex;
			Refresh2();
		}

		void OnGameChanged()
		{
			if (loading) return;
			if (busy)
			{
				loading = true;
				cbGame.SelectedIndex = prevGame;
				loading = false;
				return;
			}
			if (!EnsurePaths(Gamma))
			{
				loading = true;
				cbGame.SelectedIndex = prevGame; // cancelled: back to the game it was
				loading = false;
				return;
			}
			prevGame = cbGame.SelectedIndex;
			S.Game = Gamma ? "gamma" : "anomaly";
			S.Save();
			Refresh2();
		}

		string AskFolder(string what, string start)
		{
			using (FolderBrowserDialog d = new FolderBrowserDialog())
			{
				d.Description = what;
				d.ShowNewFolderButton = false;
				if (!string.IsNullOrEmpty(start) && Directory.Exists(start)) d.SelectedPath = start;
				return d.ShowDialog(this) == DialogResult.OK ? d.SelectedPath : null;
			}
		}

		// G.A.M.M.A: the MO2 folder first (mandatory), then Anomaly if it is not known yet; Anomaly: Anomaly if not known.
		bool EnsurePaths(bool gamma)
		{
			if (gamma && Games.CheckMo2(S.Mo2Dir) != null)
			{
				string found = Games.Find(new[] { "GAMMA", "G.A.M.M.A", "Anomaly GAMMA" }, Games.CheckMo2);
				while (true)
				{
					string dir = AskFolder("Укажи папку G.A.M.M.A — ту, где лежит ModOrganizer.exe", found.Length > 0 ? found : S.Mo2Dir);
					if (dir == null) return false;
					string err = Games.CheckMo2(dir);
					if (err == null) { S.Mo2Dir = dir; break; }
					MessageBox.Show(this, dir + ": " + err, "RayLink", MessageBoxButtons.OK, MessageBoxIcon.Warning);
				}
				if (S.Profile.Length == 0 || !Games.Profiles(S.Mo2Dir).Contains(S.Profile)) S.Profile = Games.SelectedProfile(S.Mo2Dir);
				S.Save();
			}
			if (Games.CheckAnomaly(S.AnomalyDir) != null)
			{
				string guess = gamma ? Games.GamePath(S.Mo2Dir) : "";
				if (guess.Length == 0 || Games.CheckAnomaly(guess) != null) guess = Games.Find(new[] { "Anomaly", "S.T.A.L.K.E.R. Anomaly" }, Games.CheckAnomaly);
				while (true)
				{
					string dir = AskFolder("Укажи папку Anomaly — где лежат db, gamedata, bin и fsgame.ltx", guess);
					if (dir == null) return false;
					string err = Games.CheckAnomaly(dir);
					if (err == null) { S.AnomalyDir = dir; break; }
					MessageBox.Show(this, dir + ": " + err, "RayLink", MessageBoxButtons.OK, MessageBoxIcon.Warning);
				}
				S.Save();
			}
			if (gamma)
			{
				string gp = Games.GamePath(S.Mo2Dir);
				if (gp.Length > 0 && !string.Equals(Path.GetFullPath(gp).TrimEnd('\\'), Path.GetFullPath(S.AnomalyDir).TrimEnd('\\'), StringComparison.OrdinalIgnoreCase))
					Log("! у MO2 игра в " + gp + ", а указана " + S.AnomalyDir + " — G.A.M.M.A собрана поверх другой Anomaly?");
			}
			if (string.IsNullOrEmpty(S.StandRoot))
			{
				// Owner 2026-10-24: the folder the launcher was installed into is RayLink's home - builds, scripts and the
				// stand go there. The stand is hard links to the game's files: on another volume than the game it goes to
				// <game volume>\RayLink (the builds still come into the home).
				string home = Application.StartupPath;
				string drive = Path.GetPathRoot(gamma ? Games.ModsDir(S.Mo2Dir) : S.AnomalyDir);
				if (string.Equals(Path.GetPathRoot(home), drive, StringComparison.OrdinalIgnoreCase)) S.StandRoot = home;
				else
				{
					S.StandRoot = Path.Combine(drive, "RayLink");
					Log("стенд — на диске игры (" + S.StandRoot + "): жёсткие ссылки работают только в пределах одного диска");
				}
				S.Save();
			}
			return true;
		}

		void Refresh2()
		{
			if (InvokeRequired) { BeginInvoke(new Action(Refresh2)); return; }
			loading = true;
			cbProfile.Visible = Gamma;
			cbProfile.Items.Clear();
			if (Gamma && Games.CheckMo2(S.Mo2Dir) == null)
			{
				foreach (string p in Games.Profiles(S.Mo2Dir)) cbProfile.Items.Add(p);
				int i = cbProfile.Items.IndexOf(S.Profile);
				if (i < 0 && cbProfile.Items.Count > 0) { i = 0; S.Profile = Convert.ToString(cbProfile.Items[0]); }
				cbProfile.SelectedIndex = i;
			}
			loading = false;
			string ver = Gamma ? Games.GammaVersion(S.Mo2Dir) : "";
			lblGame.Text = Gamma ? ("G.A.M.M.A" + (ver.Length > 0 ? " " + ver : "") + " · " + S.Mo2Dir + " · Anomaly " + S.AnomalyDir)
				: ("Anomaly · " + S.AnomalyDir);
			lblBuild.Text = S.Installed.Length > 0 ? S.Installed.TrimStart('v', 'b') : "—";
			lblBuildState.Text = S.Installed.Length > 0 ? "установлена " + S.Installed : "сборки нет — нажми «Обновить сборку»";
			string sd = StandDir();
			bool standReady = sd.Length > 0 && File.Exists(Path.Combine(sd, Stand.Manifest)) && File.Exists(Path.Combine(Path.Combine(sd, "bin"), Program.ExeName));
			lblStand.Text = "стенд: " + (sd.Length > 0 ? sd : "—") + (standReady ? "" : " (не собран)");
			btnPlay.Text = standReady ? "ИГРАТЬ" : "СОБРАТЬ И ИГРАТЬ";
			btnPlay.Font = new Font("Segoe UI", standReady ? 20f : 14f, FontStyle.Bold);
			tbAddress.Enabled = rbJoin.Checked;
		}

		// ---------------------------------------------------------- work in the background
		void Run(string what, Action work)
		{
			if (busy) return;
			busy = true;
			SetEnabled(false);
			Task.Factory.StartNew(delegate
			{
				try { work(); }
				catch (Exception e) { Log("! " + what + ": " + e.Message); }
				finally
				{
					busy = false;
					BeginInvoke(new Action(delegate { SetEnabled(true); Refresh2(); }));
				}
			});
		}

		void SetEnabled(bool on)
		{
			foreach (Control c in new Control[] { cbGame, cbProfile, btnUpdate, btnStand, btnPlay, btnSettings, chkSecond })
				c.Enabled = on;
		}

		void UpdateBuild()
		{
			Log("ищу сборку в " + S.Repo + " …");
			Release rel = Channel.Latest(S.Repo, S.Token);
			Log("последняя: " + rel.Name + " (" + rel.Tag + "), " + (rel.AssetSize / 1048576.0).ToString("F1") + " МБ");
			string dir = BuildDir(rel.Tag);
			if (S.Installed == rel.Tag && Directory.Exists(dir))
			{
				Log("уже установлена");
				return;
			}
			string cache = Path.Combine(Application.StartupPath, "cache");
			Directory.CreateDirectory(cache);
			string zip = Path.Combine(cache, rel.AssetName);
			Log("качаю " + rel.AssetName + " …");
			Channel.Download(rel, S.Token, zip);
			string tmp = dir + ".tmp";
			if (Directory.Exists(tmp)) Directory.Delete(tmp, true);
			ZipFile.ExtractToDirectory(zip, tmp);
			if (!File.Exists(Path.Combine(Path.Combine(Path.Combine(tmp, "Anomaly Coop - Engine"), "bin"), Program.ExeName)))
				throw new IOException("в архиве нет engine\\bin\\" + Program.ExeName + " — это не сборка RayLink");
			if (Directory.Exists(dir)) Directory.Delete(dir, true);
			Directory.Move(tmp, dir);
			S.Installed = rel.Tag;
			S.Save();
			Log("сборка " + rel.Tag + " установлена; дальше — «Собрать стенд»");
		}

		void BuildStand()
		{
			if (S.Installed.Length == 0 || !Directory.Exists(BuildDir(S.Installed))) UpdateBuild();
			string b = BuildDir(S.Installed);
			string sd = StandDir();
			List<Layer> layers = new List<Layer>();
			Layer game = new Layer();
			game.Name = "<Anomaly>"; game.Root = S.AnomalyDir; game.Kind = "game";
			layers.Add(game);
			Layer engBin = new Layer();
			engBin.Name = "<RayLink engine>"; engBin.Root = Path.Combine(Path.Combine(b, "Anomaly Coop - Engine"), "bin"); engBin.Prefix = "bin"; engBin.Copy = true; engBin.Kind = "ours";
			layers.Add(engBin);
			Layer engData = new Layer();
			engData.Name = "<RayLink engine gamedata>"; engData.Root = Path.Combine(Path.Combine(b, "Anomaly Coop - Engine"), "gamedata"); engData.Prefix = "gamedata"; engData.Copy = true; engData.Kind = "ours";
			layers.Add(engData);
			if (Gamma)
			{
				string modlist = Path.Combine(Path.Combine(Games.ProfilesDir(S.Mo2Dir), S.Profile), "modlist.txt");
				if (!File.Exists(modlist)) throw new IOException("нет " + modlist);
				string modsDir = Games.ModsDir(S.Mo2Dir);
				foreach (string m in Games.ReadModlist(modlist))
				{
					Layer l = new Layer();
					l.Name = m; l.Root = Path.Combine(modsDir, m);
					layers.Add(l);
				}
				Layer ow = new Layer();
				ow.Name = "<overwrite>"; ow.Root = Games.OverwriteDir(S.Mo2Dir); ow.Kind = "overwrite";
				layers.Add(ow);
				if (Process.GetProcessesByName("ModOrganizer").Length > 0)
					Log("! MO2 открыт: если он сейчас обновляет моды, стенд соберётся из половины");
			}
			Layer core = new Layer();
			core.Name = "<RayLink>"; core.Root = Path.Combine(Path.Combine(b, "Anomaly Coop - Core"), "gamedata"); core.Prefix = "gamedata"; core.Copy = true; core.Kind = "ours";
			layers.Add(core);
			string gammaData = Path.Combine(Path.Combine(b, "Anomaly Coop - GAMMA"), "gamedata");
			if (Gamma && Directory.Exists(gammaData))
			{
				Layer gl = new Layer();
				gl.Name = "<RayLink GAMMA>"; gl.Root = gammaData; gl.Prefix = "gamedata"; gl.Copy = true; gl.Kind = "ours";
				layers.Add(gl);
			}
			string holders = Directory.Exists(sd) ? Stand.RunningFrom(sd) : "";
			if (holders.Length > 0)
				throw new IOException("из стенда запущено: " + holders + ". Закрой игру (и второе окно), потом собирай стенд");
			Log("собираю стенд " + sd + " (" + (Gamma ? "G.A.M.M.A, профиль " + S.Profile : "Anomaly") + ", сборка " + S.Installed + ") …");
			Stand st = new Stand(Log);
			st.Materialize(layers, sd);
			st.Instances(sd, S.AnomalyDir, CleanNick(S.Nick.Length > 0 ? S.Nick : Environment.UserName));
		}

		void Play()
		{
			S.Nick = CleanNick(tbNick.Text);
			tbNick.Text = S.Nick;
			S.Save();
			string sd = StandDir();
			bool ready = File.Exists(Path.Combine(sd, Stand.Manifest)) && File.Exists(Path.Combine(Path.Combine(sd, "bin"), Program.ExeName));
			Run("запуск", delegate
			{
				if (!ready) BuildStand();
				if (Process.GetProcessesByName(Path.GetFileNameWithoutExtension(Program.ExeName)).Length > 0)
					Log("! игра уже запущена — второй раз из той же папки движок не стартует");
				Start("a", "");
			});
		}

		void Start(string inst, string coop)
		{
			string sd = StandDir();
			string exe = Path.Combine(Path.Combine(sd, inst == "a" ? "bin" : "bin_b"), Program.ExeName);
			if (!File.Exists(exe)) throw new IOException("нет " + exe + " — собери стенд");
			string nick = inst == "a" ? S.Nick : S.Nick + "_2";
			string args = ("-fsltx fsgame_" + inst + ".ltx -smap2048 -coop_nick " + nick + " " + coop).Trim();
			ProcessStartInfo psi = new ProcessStartInfo(exe, args);
			psi.WorkingDirectory = sd;
			psi.UseShellExecute = false;
			Process p = Process.Start(psi);
			Log("запущено окно " + inst + " (pid " + p.Id + "): " + args + "; лог: " + Path.Combine(sd, "appdata_" + inst + "\\logs"));
		}

		void ShowSettings()
		{
			using (Form f = new Form())
			{
				f.Text = "Настройки RayLink";
				f.ClientSize = new Size(560, 300);
				f.FormBorderStyle = FormBorderStyle.FixedDialog;
				f.StartPosition = FormStartPosition.CenterParent;
				f.BackColor = Bg;
				f.ForeColor = Fg;
				f.Font = Font;
				TextBox repo = new TextBox(), token = new TextBox(), stand = new TextBox();
				int y = 16;
				foreach (KeyValuePair<string, TextBox> kv in new[] {
					new KeyValuePair<string, TextBox>("Репозиторий сборок (владелец/имя)", repo),
					new KeyValuePair<string, TextBox>("Ключ GitHub (только для закрытого репозитория; хранится зашифрованным)", token),
					new KeyValuePair<string, TextBox>("Папка стендов (тот же диск, что игра и моды)", stand) })
				{
					Label l = new Label();
					l.Text = kv.Key; l.AutoSize = true; l.ForeColor = Dim; l.Location = new Point(16, y);
					f.Controls.Add(l);
					kv.Value.Location = new Point(16, y + 22);
					kv.Value.Size = new Size(528, 26);
					kv.Value.BackColor = Panel2; kv.Value.ForeColor = Fg; kv.Value.BorderStyle = BorderStyle.FixedSingle;
					f.Controls.Add(kv.Value);
					y += 62;
				}
				repo.Text = S.Repo;
				token.UseSystemPasswordChar = true;
				token.Text = S.TokenProtected.Length > 0 ? "********" : "";
				stand.Text = S.StandRoot;
				Button paths = new Button(), ok = new Button();
				paths.Text = "Сменить пути к играм";
				paths.Location = new Point(16, 220); paths.Size = new Size(220, 32); paths.FlatStyle = FlatStyle.Flat;
				paths.Click += delegate { S.Mo2Dir = Gamma ? "" : S.Mo2Dir; S.AnomalyDir = ""; EnsurePaths(Gamma); Refresh2(); };
				ok.Text = "Сохранить";
				ok.Location = new Point(424, 250); ok.Size = new Size(120, 34); ok.FlatStyle = FlatStyle.Flat;
				ok.BackColor = Sand; ok.ForeColor = Color.FromArgb(20, 18, 12);
				ok.Click += delegate
				{
					if (repo.Text.Trim().Length > 0) S.Repo = repo.Text.Trim();
					if (token.Text != "********") S.Token = token.Text.Trim();
					if (stand.Text.Trim().Length > 0) S.StandRoot = stand.Text.Trim();
					S.Save();
					f.Close();
				};
				f.Controls.Add(paths);
				f.Controls.Add(ok);
				f.ShowDialog(this);
			}
			Refresh2();
		}
	}
}
