use chrono::Duration;
use reqwest::{
    blocking::{Client, Response},
    Error,
};
use std::{
    error,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
    process::Command,
    str,
};

const APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);
pub struct SignalManager {
    path: String,
    messages_folder: String,
    account_number: String,
    config_path: String,
}

impl SignalManager {
    pub fn new(messages_folder: String) -> Self {
        SignalManager {
            messages_folder,
            account_number: "+919074221997".to_string(),
            config_path: "/home/jerin/.local/share/signal-cli".to_string(),
            path: SignalManager::get_signal_path(),
        }
    }

    pub fn send_messages(&self) {
        let path = Path::new(&self.messages_folder).join("to-send");
        if !path.exists() {
            return;
        }
        for top_level_entry in path.read_dir().unwrap() {
            if let Ok(dir_entry) = top_level_entry {
                if !dir_entry.file_type().unwrap().is_dir() {
                    continue;
                }
                let dir_path = dir_entry.path();
                let client_number = dir_path.file_name().unwrap();
                for entry in dir_path.read_dir().unwrap() {
                    if let Ok(file_entry) = entry {
                        let file_path = file_entry.path();
                        let extension = file_path.extension().unwrap();
                        let stem = file_path.file_stem().unwrap().to_str().unwrap();
                        if extension == "lock" {
                            continue;
                        }
                        if let Err(_) = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create_new(true)
                            .open(&dir_path.join(stem.to_owned() + ".lock"))
                        {
                            continue;
                        }
                        match extension.to_str().unwrap() {
                            "signalmessage" => self.send_message(
                                fs::read_to_string(&file_path).unwrap(),
                                client_number.to_str().unwrap().to_owned(),
                            ),
                            "signalattachment" => self
                                .send_attachment(
                                    Path::new(&fs::read_to_string(&file_path).unwrap()),
                                    client_number.to_str().unwrap().to_owned(),
                                )
                                .unwrap(),
                            "signalreply" => {
                                let filecontent = fs::read_to_string(&file_path).unwrap();
                                let (timestamp, message) = filecontent.split_once('\n').unwrap();
                                self.reply_to_message(
                                    message.to_owned(),
                                    timestamp,
                                    client_number.to_str().unwrap().to_owned(),
                                );
                            }
                            _ => (),
                        };
                        fs::remove_file(&file_path).unwrap();
                        fs::remove_file(&dir_path.join(stem.to_owned() + ".lock")).unwrap();
                    }
                }
            }
        }
    }

    pub fn receive_messages(&self) {
        println!("Looking for messages...");
        let mut command = self.get_signal_command();
        let out = match command.arg("receive").output() {
            Ok(o) => o,
            Err(e) => {
                println!("{}", e);
                return;
            }
        };
        println!("Received messages.");
        let outstring = str::from_utf8(&out.stdout).unwrap();
        let v: serde_json::Value = serde_json::from_str(outstring).unwrap();
        let mut path = Path::new(&self.messages_folder)
            .join("received")
            .join(v["envelope"]["sourceNumber"].to_string().trim_matches('"'));
        fs::create_dir_all(&path).unwrap();
        let timestamp = v["envelope"]["dataMessage"]["timestamp"].to_string();
        let timestamp = timestamp.trim_matches('"');
        path = path.join(timestamp.to_owned() + ".lock");
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        path.set_extension("signalmessage");
        let mut messagefile = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        messagefile
            .write_all(
                v["envelope"]["dataMessage"]["message"]
                    .to_string()
                    .trim_matches('"')
                    .as_bytes(),
            )
            .unwrap();
        path.set_extension("lock");
        fs::remove_file(path).unwrap();
    }

    fn get_signal_command(&self) -> Command {
        let mut command = Command::new(&self.path);
        command
            .arg("--config")
            .arg(&self.config_path)
            .arg("-a")
            .arg(&self.account_number);
        command
    }

    fn send_message(&self, message: String, client_number: String) {
        let mut command = self.get_signal_command();
        command
            .arg("send")
            .arg(client_number)
            .arg("-m")
            .arg(&message)
            .output()
            .expect("sending failed");
        println!("Message sent: {}", message);
    }

    fn send_attachment(
        &self,
        file_path: &Path,
        client_number: String,
    ) -> Result<(), Box<dyn error::Error>> {
        let mut i = 0;
        loop {
            let mut command = self.get_signal_command();
            match command
                .arg("send")
                .arg(&client_number)
                .arg("-a")
                .arg(file_path)
                .output()
            {
                Err(e) => {
                    i += 1;
                    println!("{}", e);
                    std::thread::sleep(Duration::seconds(5).to_std().unwrap());
                    if i == 5 {
                        self.send_message(
                            "Sending attachment keeps failing".to_string(),
                            client_number,
                        );
                        return Err("Sending attachment failed!".into());
                    }
                }
                Ok(o) => {
                    println!("{:?}", o);
                    println!("Attachment sent: {}", &file_path.display());
                    return Ok(());
                }
            }
        }
    }

    fn reply_to_message(
        &self,
        reply: String,
        message_timestamp_to_quote: &str,
        client_number: String,
    ) {
        let mut command = self.get_signal_command();
        command
            .arg("send")
            .arg(client_number)
            .arg("-m")
            .arg(&reply)
            .arg("--quote-timestamp")
            .arg(message_timestamp_to_quote)
            .output()
            .expect("sending failed");
        println!("Message sent: {}", reply);
    }

    fn get_signal_path() -> String {
        let client = Client::builder()
            .cookie_store(true)
            .user_agent(APP_USER_AGENT)
            .build()
            .unwrap();
        let (download, version) = SignalManager::signal_version_check();
        if download {
            let res = SignalManager::get_url_response(
                &client,
                format!(
                    "https://github.com/AsamK/signal-cli/releases/download/v{}/signal-cli-{}.tar.gz",
                    &version, &version
                )
                .as_str(),
            );
            let res = res.unwrap().bytes().unwrap();
            std::fs::write(
                "/home/jerin/RustProjects/etender/downloaded_file.tar.gz",
                &res,
            )
            .expect("Reference proteome download failed for {file_name}");
            let tar_gz =
                File::open("/home/jerin/RustProjects/etender/downloaded_file.tar.gz").unwrap();
            let tar = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(tar);
            archive.unpack("/opt").unwrap();
            SignalManager::replace_with_correct_libsignal(&version);
        }
        format!("/opt/signal-cli-{}/bin/signal-cli", version)
    }

    fn get_url_response(client: &Client, url: &str) -> Result<Response, Error> {
        client
            .get(url)
            .timeout(Duration::seconds(60).to_std().unwrap())
            .send()
    }

    fn signal_version_check() -> (bool, String) {
        let client = Client::builder()
            .cookie_store(true)
            .user_agent(APP_USER_AGENT)
            .build()
            .unwrap();
        let res = SignalManager::get_url_response(
            &client,
            "https://api.github.com/repos/AsamK/signal-cli/releases/latest",
        )
        .unwrap();
        let pq = res.json::<serde_json::Value>().unwrap();
        let q = pq["tag_name"].to_string();
        let p: Vec<&str> = (&q)
            .split(|x| x == '"' || x == 'v')
            .filter(|x| !x.is_empty())
            .collect();
        let mut version_number = String::new();
        let mut new_version_found = false;
        if !p.is_empty() {
            version_number = p[0].to_string();
            let file =
                File::open("/home/jerin/RustProjects/etender/signal_version_number.txt").unwrap();
            let reader = BufReader::new(file);
            let line = reader.lines().flatten().last().unwrap();
            println!("{}", line);
            if version_number != line {
                println!("Downloading new signal version.");
                fs::write(
                    "/home/jerin/RustProjects/etender/signal_version_number.txt",
                    &version_number,
                )
                .unwrap();
                new_version_found = true;
            }
        }
        (new_version_found, version_number)
    }

    fn get_libsignal_version() -> (bool, String) {
        let client = Client::builder()
            .cookie_store(true)
            .user_agent(APP_USER_AGENT)
            .build()
            .unwrap();
        let res = SignalManager::get_url_response(
            &client,
            "https://api.github.com/repos/exquo/signal-libs-build/releases/latest",
        )
        .unwrap();
        let pq = res.json::<serde_json::Value>().unwrap();
        let q = pq["tag_name"].to_string();
        let q = q.split_once("_").unwrap().1.split_once('"').unwrap().0;
        let mut new_version_found = false;
        let version_number = q.to_string();
        let file =
            File::open("/home/jerin/RustProjects/etender/libsignal_version_number.txt").unwrap();
        let reader = BufReader::new(file);
        let line = reader.lines().flatten().last().unwrap();
        println!("{}", line);
        if version_number != line {
            println!("Downloading new libsignal version.");
            fs::write(
                "/home/jerin/RustProjects/etender/libsignal_version_number.txt",
                &version_number,
            )
            .unwrap();
            new_version_found = true;
        }
        (new_version_found, version_number)
    }

    fn replace_with_correct_libsignal(signal_version: &str) {
        let client = Client::builder()
            .cookie_store(true)
            .user_agent(APP_USER_AGENT)
            .build()
            .unwrap();
        let (download, version) = SignalManager::get_libsignal_version();
        if download {
            let url = format!("https://github.com/exquo/signal-libs-build/releases/download/libsignal_{}/libsignal_jni.so-{}-aarch64-unknown-linux-gnu.tar.gz", &version, &version);
            println!("{}", url);
            let res = SignalManager::get_url_response(&client, &url);
            let res = res.unwrap().bytes().unwrap();
            std::fs::write(
                "/home/jerin/RustProjects/etender/downloaded_file.tar.gz",
                &res,
            )
            .expect("Reference proteome download failed for {file_name}");
            let tar_gz =
                File::open("/home/jerin/RustProjects/etender/downloaded_file.tar.gz").unwrap();
            let tar = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(tar);
            archive.unpack("/opt").unwrap();
        }
        let paths = fs::read_dir("/opt/signal-cli-".to_string() + signal_version + "/lib").unwrap();
        for path in paths {
            let pathstring = path.unwrap().path();
            let fullname = pathstring.file_name().unwrap().to_str().unwrap();
            if fullname.len() > 16 {
                let newname = &fullname[..16];
                if newname == "libsignal-client" {
                    SignalManager::rezip_file(pathstring.to_str().unwrap());
                    break;
                }
            }
        }
    }

    fn rezip_file(libsignal_path: &str) {
        let newlibsignal_path = "/opt/signal-cli-0.13.4/lib/2libsignal-client-0.47.0.jar";
        let file = File::open(&libsignal_path).unwrap();
        let mut newfile = File::create(newlibsignal_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let filenames = archive
            .file_names()
            .map(|x| x.to_string())
            .collect::<Vec<String>>();
        let mut zip = zip::ZipWriter::new(&mut newfile);
        let options = zip::write::FileOptions::default();
        for filename in filenames {
            let mut zip_file = archive.by_name(&filename).unwrap();
            let mut writer: Vec<u8> = vec![];
            if filename == "libsignal_jni.so".to_owned() {
                println!("{}", &filename);
                let libspath = "/opt/libsignal_jni.so";
                let mut libsfile = File::open(&libspath).unwrap();
                io::copy(&mut libsfile, &mut writer).unwrap();
            } else {
                io::copy(&mut zip_file, &mut writer).unwrap();
            }
            zip.start_file(filename, options).unwrap();
            zip.write_all(&writer).unwrap();
        }
        zip.finish().unwrap();
        fs::remove_file(libsignal_path).unwrap();
        fs::rename(newlibsignal_path, libsignal_path).unwrap();
    }
}
