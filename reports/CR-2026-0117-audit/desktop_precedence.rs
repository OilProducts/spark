use spark_desktop::desktop_core::{DesktopPaths,DesktopServerSettings,bootstrap_desktop_runtime};
fn main(){
 let root=std::path::PathBuf::from(std::env::args().nth(1).unwrap());
 let paths=DesktopPaths::new(root.join("data"),root.join("config"));
 let boot=bootstrap_desktop_runtime(&paths,&DesktopServerSettings::default()).unwrap();
 let mut capture=spark_storage::settings::read_execution_configuration(&boot.settings.config_dir,&std::env::vars().collect::<std::collections::BTreeMap<String,String>>()).unwrap();
 println!("resolved_before_startup_retention={:?}",capture.agents.native.claude_config_dir);
 capture.retain_startup_settings(&boot.settings);
 println!("captured_after_startup_retention={:?}",capture.agents.native.claude_config_dir);
 println!("desktop_startup={:?}",boot.settings.agents.native.claude_config_dir);
 assert_eq!(capture.agents.native.claude_config_dir,Some("/tmp/audit-claude-config".to_owned()),"Desktop must retain the environment-selected home at startup");
}
