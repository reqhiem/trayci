fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint_build::compile("ui/app.slint")?;
    embed_resource::compile("resources/icons/trayci.rc", embed_resource::NONE)
        .manifest_required()?;
    Ok(())
}
