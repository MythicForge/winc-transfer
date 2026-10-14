use crate::store::use_store;
use crate::types::Role;
use leptos::prelude::*;

#[component]
pub fn RolePicker() -> impl IntoView {
    let store = use_store();
    view! {
        <div class="picker">
            <div class="picker__inner">
                <img src="/WINCI.svg" alt="WINCI" class="picker__logo" style="width: 72px; height: 72px" />
                <div class="eyebrow">"WINCI · PC-to-PC data crossing"</div>
                <h1>"Move a whole PC" <br /> "across one wire."</h1>
                <p class="picker__lead">
                    "Connect the two computers with a Thunderbolt / USB4 cable — or use any Wi-Fi or "
                    "Ethernet network by entering the new PC's IP address. WINCI carries your files and "
                    "browser data straight across — no cloud, no accounts, end-to-end encrypted."
                </p>
                <div class="picker__grid">
                    <div class="role-card">
                        <div class="role-card__glyph">"On the OLD PC"</div>
                        <h3>"Send my data"</h3>
                        <p>"This computer holds the files. Pick what to carry over and stream it to the new machine."</p>
                        <button class="btn btn--primary" on:click=move |_| store.set_role(Role::Send)>
                            "Start sending →"
                        </button>
                    </div>
                    <div class="role-card">
                        <div class="role-card__glyph">"On the NEW PC"</div>
                        <h3>"Receive data"</h3>
                        <p>"This computer is the destination. Show a pairing code to the old PC and stand by."</p>
                        <button class="btn" on:click=move |_| store.set_role(Role::Receive)>
                            "Wait for a sender"
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}
